//! Find in Files (`IntelliJ` Cmd/Ctrl+Shift+F) from a native editor:
//! project-wide text search with match case, whole word, regex, and a
//! file mask. Results group by file with line previews; a click opens the
//! file at the match. Each query runs on the filesystem pool, at most
//! [`RESULT_LIMIT`] hits, and a newer query cancels the one in flight.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod search;
mod view;

pub(crate) use search::{SearchOutcome, SearchRequest};

/// Hits kept per search; more are reported as truncated.
pub(crate) const RESULT_LIMIT: usize = 1000;

type SearchAnswer = (u64, Result<SearchOutcome, String>);

/// The Find in Files window and its search in flight.
pub(crate) struct FindInFiles {
    pub root: PathBuf,
    pub query: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    /// Comma- or space-separated globs (`*.rs, src/**`); empty is all.
    pub mask: String,
    /// Focus the query field on the next paint.
    pub focus: bool,
    /// The query the latest search ran for (unchanged input never re-runs).
    searched: Option<crate::search::Query>,
    generation: u64,
    /// Cancel flag of the search in flight.
    cancel: Arc<AtomicBool>,
    answers: Option<tokio::sync::mpsc::UnboundedReceiver<SearchAnswer>>,
    pub running: bool,
    pub outcome: Option<Result<SearchOutcome, String>>,
}

impl FindInFiles {
    pub fn new(root: PathBuf, query: String) -> Self {
        Self {
            root,
            query,
            case: false,
            word: false,
            regex: false,
            mask: String::new(),
            focus: true,
            searched: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            answers: None,
            running: false,
            outcome: None,
        }
    }

    /// The query as the search worker reads it.
    pub fn search_query(&self) -> crate::search::Query {
        crate::search::Query {
            text: self.query.clone(),
            match_case: self.case,
            whole_word: self.word,
            use_regex: self.regex,
            include: self.mask.clone(),
            exclude: String::new(),
        }
    }

    /// A new search when the input changed since the last one: cancels
    /// the search in flight and returns the request plus its cancel flag
    /// and generation. `None` when nothing changed or the query is empty.
    pub fn next_search(&mut self) -> Option<(SearchRequest, Arc<AtomicBool>, u64)> {
        let query = self.search_query();
        if self.searched.as_ref() == Some(&query) {
            return None;
        }
        self.searched = Some(query.clone());
        self.cancel.store(true, Ordering::Relaxed);
        self.generation = self.generation.wrapping_add(1);
        self.answers = None;
        if query.is_empty() {
            self.running = false;
            self.outcome = None;
            return None;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        let request = SearchRequest {
            root: self.root.clone(),
            query,
            limit: RESULT_LIMIT,
        };
        Some((request, Arc::clone(&self.cancel), self.generation))
    }

    /// Take a finished answer for the current generation; older ones
    /// (cancelled queries) are dropped.
    pub fn accept(&mut self, generation: u64, result: Result<SearchOutcome, String>) {
        if generation != self.generation {
            return;
        }
        self.running = false;
        if result.as_ref().is_ok_and(|outcome| outcome.cancelled) {
            return;
        }
        self.outcome = Some(result);
    }

    fn poll(&mut self) {
        let Some(answers) = self.answers.as_mut() else {
            return;
        };
        match answers.try_recv() {
            Ok((generation, result)) => {
                self.answers = None;
                self.accept(generation, result);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.answers = None;
                self.running = false;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
        }
    }
}

impl Drop for FindInFiles {
    /// Closing the window stops its search.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl App {
    /// Open (or refocus) Find in Files for the project of the editor that
    /// asked, seeded with its one-line selection.
    pub(crate) fn open_find_in_files(&mut self, project: Option<&str>, seed: String) {
        let root = project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .or_else(|| self.selected_project())
            .map(|project| project.path.clone());
        let Some(root) = root else {
            self.info = Some("Find in Files needs an open project".into());
            return;
        };
        match self.find_in_files.as_mut().filter(|find| find.root == root) {
            Some(find) => {
                if !seed.is_empty() {
                    find.query = seed;
                }
                find.focus = true;
            }
            None => self.find_in_files = Some(FindInFiles::new(root, seed)),
        }
    }

    /// Start a search when the input changed, and take finished answers.
    pub(crate) fn drive_find_in_files(&mut self) {
        let Some(find) = self.find_in_files.as_mut() else {
            return;
        };
        find.poll();
        let Some((request, cancel, generation)) = find.next_search() else {
            return;
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        find.answers = Some(rx);
        let service = self.services.clone();
        let submitted = self.services.handle().submit(
            OperationContext::new(
                "find-in-files",
                request.root.display().to_string(),
                Policy::ReplaceableRead,
            ),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || anyhow::Ok(search::run(&request, &cancel)))
                    .await;
                let result = result.unwrap_or_else(|error| Err(format!("{error:#}")));
                let _ = tx.send((generation, result));
                Ok(Vec::new())
            },
        );
        if let Err(error) = submitted {
            find.accept(generation, Err(format!("Could not search: {error}")));
        }
    }
}

#[cfg(test)]
mod tests;
