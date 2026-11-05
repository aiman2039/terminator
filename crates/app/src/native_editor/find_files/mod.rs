//! Find in Files (`IntelliJ` Cmd/Ctrl+Shift+F): text search over the
//! project or one folder, with match case, whole word, regex, a file mask,
//! and Exclude globs. It opens from any pane, the palette, or an Explorer
//! folder. Results stream in grouped by file; the arrows pick one, a
//! preview shows the lines around it, and Enter opens the file at the
//! match. Each query runs on the filesystem pool, at most [`RESULT_LIMIT`]
//! hits, and a newer query cancels the one in flight. Replace mode
//! (Cmd/Ctrl+Shift+R) lives in [`replace`].

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod cursor;
mod open;
mod preview;
mod replace;
mod search;
mod view;

pub(crate) use open::{FindScope, explorer_has_focus, note_explorer_focus};
pub(crate) use search::{FileHits, LineHit, SearchRequest};

/// Hits kept per search; more ask to refine it.
pub(crate) const RESULT_LIMIT: usize = 1000;

/// What the worker sends, tagged with its search generation.
pub(crate) enum SearchEvent {
    /// One more file with hits, in walk order.
    File(FileHits),
    /// The walk ended; `truncated` when the hit limit stopped it.
    Done { truncated: bool },
    /// A bad regex, or a job that could not run.
    Failed(String),
}

type SearchAnswer = (u64, SearchEvent);

/// The Find in Files window, its results, and the search in flight.
pub(crate) struct FindInFiles {
    pub root: PathBuf,
    /// A folder below `root` that narrows the search.
    pub folder: Option<PathBuf>,
    pub query: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    /// Comma- or space-separated globs (`*.rs, src/**`); empty is all.
    pub mask: String,
    /// Globs to leave out (`target/**, *.min.js`).
    pub exclude: String,
    /// The window shows. Closing it keeps the query for the next open.
    pub shown: bool,
    /// Focus the query field on the next paint; `select_query` also
    /// selects its text, so typing replaces it.
    pub focus: bool,
    pub select_query: bool,
    /// The widget that had keyboard focus before the window opened; Esc
    /// gives focus back to it.
    pub return_focus: Option<egui::Id>,
    /// The query and folder the latest search ran for (unchanged input
    /// never searches again).
    searched: Option<(crate::search::Query, PathBuf)>,
    generation: u64,
    /// Cancel flag of the search in flight.
    cancel: Arc<AtomicBool>,
    answers: Option<tokio::sync::mpsc::UnboundedReceiver<SearchAnswer>>,
    pub running: bool,
    pub files: Vec<FileHits>,
    /// Hits in `files`.
    pub total: usize,
    pub truncated: bool,
    pub error: Option<String>,
    /// Keyboard cursor: the n-th hit in list order.
    pub selected: usize,
    /// Scroll the list to the cursor on the next paint.
    pub reveal: bool,
    /// The list's scroll offset at the last paint.
    pub scroll: f32,
    /// Highlighted preview of the cursor's hit, rebuilt when it moves.
    preview: Option<preview::Preview>,
    /// Replace mode: the replacement, checkboxes, and the replace running.
    pub replace: replace::ReplaceState,
}

impl FindInFiles {
    pub fn new(scope: FindScope, query: String) -> Self {
        Self {
            root: scope.root,
            folder: scope.folder,
            query,
            case: false,
            word: false,
            regex: false,
            mask: String::new(),
            exclude: String::new(),
            shown: true,
            focus: true,
            select_query: true,
            return_focus: None,
            searched: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            answers: None,
            running: false,
            files: Vec::new(),
            total: 0,
            truncated: false,
            error: None,
            selected: 0,
            reveal: false,
            scroll: 0.0,
            preview: None,
            replace: replace::ReplaceState::default(),
        }
    }

    /// Show the window again for `scope`. A non-empty `seed` replaces the
    /// query; otherwise the last query stays, selected.
    pub fn reopen(&mut self, scope: FindScope, seed: String) {
        self.root = scope.root;
        self.folder = scope.folder;
        if !seed.is_empty() {
            self.query = seed;
        }
        self.shown = true;
        self.focus = true;
        self.select_query = true;
    }

    /// Close the window: stop the search and drop its results, but keep
    /// the query, toggles, and globs for the next open.
    pub fn hide(&mut self) {
        self.shown = false;
        self.return_focus = None;
        self.cancel.store(true, Ordering::Relaxed);
        self.answers = None;
        self.running = false;
        self.searched = None;
        self.clear_results();
    }

    fn clear_results(&mut self) {
        self.files.clear();
        self.total = 0;
        self.truncated = false;
        self.error = None;
        self.selected = 0;
        // New results start at the top.
        self.reveal = true;
        self.scroll = 0.0;
        self.preview = None;
        self.replace.forget_results();
    }

    /// Where the walk starts: the folder, or the whole root.
    pub fn walk_folder(&self) -> &Path {
        self.folder.as_deref().unwrap_or(&self.root)
    }

    /// The query as the search worker reads it.
    pub fn search_query(&self) -> crate::search::Query {
        crate::search::Query {
            text: self.query.clone(),
            match_case: self.case,
            whole_word: self.word,
            use_regex: self.regex,
            include: self.mask.clone(),
            exclude: self.exclude.clone(),
        }
    }

    /// A new search when the input changed since the last one: cancels
    /// the search in flight, clears the results, and returns the request
    /// plus its cancel flag and generation. `None` when nothing changed or
    /// the query is empty.
    pub fn next_search(&mut self) -> Option<(SearchRequest, Arc<AtomicBool>, u64)> {
        let key = (self.search_query(), self.walk_folder().to_path_buf());
        if self.searched.as_ref() == Some(&key) {
            return None;
        }
        self.searched = Some(key.clone());
        self.cancel.store(true, Ordering::Relaxed);
        self.generation = self.generation.wrapping_add(1);
        self.answers = None;
        self.clear_results();
        let (query, folder) = key;
        if query.is_empty() {
            self.running = false;
            return None;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        let request = SearchRequest {
            root: self.root.clone(),
            folder,
            query,
            limit: RESULT_LIMIT,
        };
        Some((request, Arc::clone(&self.cancel), self.generation))
    }

    /// Take one worker event for the current generation; events of older
    /// (cancelled) searches are dropped.
    pub fn accept(&mut self, generation: u64, event: SearchEvent) {
        if generation != self.generation {
            return;
        }
        match event {
            SearchEvent::File(file) => {
                self.total = self.total.saturating_add(file.hits.len());
                self.files.push(file);
            }
            SearchEvent::Done { truncated } => {
                self.running = false;
                self.truncated = truncated;
            }
            SearchEvent::Failed(error) => {
                self.running = false;
                self.error = Some(error);
            }
        }
    }

    /// Take everything the worker sent since the last frame.
    fn poll(&mut self) {
        let Some(mut answers) = self.answers.take() else {
            return;
        };
        loop {
            match answers.try_recv() {
                Ok((generation, event)) => self.accept(generation, event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.running = false;
                    return;
                }
            }
        }
        if self.running {
            self.answers = Some(answers);
        }
    }
}

impl Drop for FindInFiles {
    /// Dropping the window state stops its search.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl App {
    /// Start a search when the input changed, and take what the worker
    /// sent so far.
    pub(crate) fn drive_find_in_files(&mut self) {
        let Some(find) = self.find_in_files.as_mut().filter(|find| find.shown) else {
            return;
        };
        find.poll();
        let Some((request, cancel, generation)) = find.next_search() else {
            return;
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        find.answers = Some(rx);
        let service = self.services.clone();
        let failed = tx.clone();
        let submitted = self.services.handle().submit(
            search_context(&request.folder),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || {
                        let mut sink =
                            |file: FileHits| tx.send((generation, SearchEvent::File(file))).is_ok();
                        let end = match search::run(&request, &cancel, &mut sink) {
                            Ok(finish) => SearchEvent::Done {
                                truncated: finish.truncated,
                            },
                            Err(error) => SearchEvent::Failed(error),
                        };
                        let _ = tx.send((generation, end));
                        anyhow::Ok(())
                    })
                    .await;
                if let Err(error) = result {
                    let _ = failed.send((generation, SearchEvent::Failed(format!("{error:#}"))));
                }
                Ok(Vec::new())
            },
        );
        if let Err(error) = submitted {
            find.accept(
                generation,
                SearchEvent::Failed(format!("Could not search: {error}")),
            );
        }
    }
}

/// The search job's schedule. A long search in a big tree runs until it
/// ends, its limit stops it, or a new query or a closed window cancels it:
/// no read deadline cuts it off while it still streams results.
fn search_context(folder: &Path) -> OperationContext {
    let mut context = OperationContext::new(
        "find-in-files",
        folder.display().to_string(),
        Policy::ReplaceableRead,
    );
    context.deadline = None;
    context
}

#[cfg(test)]
mod tests;
