//! Git, diff review, directory refresh, and search results.

use std::path::PathBuf;

use crate::*;

/// Explicit diff actions whose undo snapshots each tab retains.
const DIFF_UNDO_LIMIT: usize = 16;

type DirectoryEntries = Result<Vec<terminator_git::Entry>, services::DirectoryError>;

pub(super) struct DirectoryPreviewInput {
    pub(super) generation: u64,
    pub(super) cwd: PathBuf,
    pub(super) path: PathBuf,
    pub(super) entries: DirectoryEntries,
}

pub(super) struct RefreshInput {
    pub(super) generation: u64,
    pub(super) context: services::ContextData,
    pub(super) directories: Vec<(PathBuf, DirectoryEntries)>,
    pub(super) fallback: bool,
}

impl App {
    pub(super) fn handle_diff(&mut self, key: String, result: Result<diff::DiffDocument, String>) {
        self.loading.remove(&key);
        match result {
            Ok(document) => {
                let review = self.diff_reviews.entry(key.clone()).or_default();
                review.context = document.context;
                review.reloaded(document.hunks.len());
                self.diffs.insert(key, Ok(std::sync::Arc::new(document)));
            }
            Err(error) if self.diffs.get(&key).is_some_and(Result::is_ok) => {
                self.error.set(format!(
                    "Diff refresh failed; showing the previous view: {error}"
                ));
            }
            Err(error) => {
                self.diffs.insert(key, Err(error));
            }
        }
    }

    pub(super) fn handle_diff_action_failed(&mut self, key: String, error: String) {
        let review = self.diff_reviews.entry(key).or_default();
        review.action_finished();
        review.saving_edits = false;
        review.message = Some(error);
    }

    pub(super) fn handle_diff_action(
        &mut self,
        tab: Tab,
        undoing: bool,
        result: Result<diff::actions::Outcome, String>,
    ) {
        let key = tab.key();
        let review = self.diff_reviews.entry(key.clone()).or_default();
        review.action_finished();
        match result {
            Ok(outcome) => {
                if review.saving_edits {
                    review.buffer = None;
                }
                if undoing {
                    review.undo.pop();
                }
                // A discard rewrote the working tree: offer Undo at once.
                review.offer_undo = outcome
                    .undo
                    .as_ref()
                    .is_some_and(|undo| undo.worktree() && !review.saving_edits);
                if let Some(undo) = outcome.undo {
                    if review.undo.len() == DIFF_UNDO_LIMIT {
                        review.undo.remove(0);
                    }
                    review.undo.push(undo);
                }
                review.message = Some(outcome.message);
            }
            Err(error) => {
                review.offer_undo = false;
                review.message = Some(error);
            }
        }
        review.saving_edits = false;
        if let Tab::Diff { cwd, path, .. } = tab {
            self.reload_open_diffs(&cwd, &path);
            self.refresh_request = None;
        }
    }

    /// A gutter Stage finished: report it, then refresh markers, open
    /// diffs of the file, and the Git sidebar.
    pub(super) fn handle_gutter_staged(
        &mut self,
        path: &std::path::Path,
        result: Result<String, String>,
    ) {
        match result {
            Ok(message) => self.info = Some(message),
            Err(error) => self.error.set(format!("Stage failed: {error}")),
        }
        self.git_base_generation = self.git_base_generation.wrapping_add(1);
        if let Some(cwd) = self.gutter_diff_cwd(path) {
            self.reload_open_diffs(&cwd, path);
        }
        self.refresh_request = None;
    }

    /// Reloads the staged and unstaged diffs of a file that are open.
    fn reload_open_diffs(&mut self, cwd: &std::path::Path, path: &std::path::Path) {
        for staged in [false, true] {
            let tab = Tab::Diff {
                remote: None,
                cwd: cwd.to_path_buf(),
                path: path.to_path_buf(),
                staged,
            };
            let key = tab.key();
            if self.diffs.contains_key(&key) {
                self.loading.insert(key.clone());
                let view = self.diff_view_for(&key, None);
                let _ = self.jobs.send(Job::DiffContext(tab, view));
            }
        }
    }

    pub(super) fn handle_directory_preview(&mut self, input: DirectoryPreviewInput) {
        let DirectoryPreviewInput {
            generation,
            cwd,
            path,
            entries,
        } = input;
        if generation != self.refresh_generation
            || !self
                .refresh_request
                .as_ref()
                .is_some_and(|request| request.cwd == cwd)
            || self.dirs.contains_key(&path)
        {
            return;
        }
        self.sidebar_cache.get_mut().clear_files();
        match entries {
            Ok(entries) => {
                self.directory_errors.remove(&path);
                self.dirs.insert(path, entries);
            }
            Err(error) => {
                if path == cwd {
                    self.note_missing_root(&path, &error);
                }
                self.directory_errors.insert(path, error);
            }
        }
    }

    pub(super) fn handle_refresh(&mut self, input: RefreshInput) {
        let RefreshInput {
            generation,
            context,
            directories,
            fallback,
        } = input;
        if generation != self.refresh_generation
            || !self
                .refresh_request
                .as_ref()
                .is_some_and(|r| r.cwd == context.cwd)
        {
            return;
        }
        let root = context.cwd.clone();
        let diff_prints = self.diff_fingerprints();
        let context = match self.context.take() {
            Some(mut previous)
                if previous.cwd == context.cwd
                    && previous.root.is_some()
                    && (context.root.is_none() || context.error.is_some()) =>
            {
                previous.error = context.error.or_else(|| {
                    Some("Repository unavailable; showing the last successful status".into())
                });
                previous
            }
            _ => context,
        };
        self.sidebar_cache.get_mut().clear_files();
        self.context = Some(context);
        self.refresh_stale_diffs(&diff_prints);
        // HEAD may have moved (commit, checkout): gutter
        // bases re-read on their next paint.
        self.git_base_generation = self.git_base_generation.wrapping_add(1);
        self.watch_fallback = fallback;
        for (path, entries) in directories {
            if path == root
                && let Err(error) = &entries
            {
                self.note_missing_root(&path, error);
            }
            self.apply_refreshed_directory(path, entries);
        }
    }

    fn apply_refreshed_directory(&mut self, path: PathBuf, entries: DirectoryEntries) {
        match entries {
            Ok(entries) => {
                #[cfg(feature = "test-support")]
                if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                    crate::console::Console::err(format_args!(
                        "Directory refresh succeeded: entries={}",
                        entries.len()
                    ));
                }
                self.directory_errors.remove(&path);
                self.dirs.insert(path, entries);
            }
            Err(error) => {
                #[cfg(feature = "test-support")]
                if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                    crate::console::Console::err(format_args!(
                        "Directory refresh failed: kind={:?} cached_entries={}",
                        error.kind,
                        self.dirs.get(&path).map_or(0, Vec::len)
                    ));
                }
                self.directory_errors.insert(path, error);
            }
        }
    }

    pub(super) fn handle_workspace(
        &mut self,
        root: &std::path::Path,
        op: workspace_ops::Op,
        result: Result<workspace_ops::Report, String>,
    ) {
        if let workspace_ops::Op::Commit { .. } = op {
            self.handle_commit_reply(result);
            return;
        }
        if self.long_run_cancelled(root, &op, &result) {
            return;
        }
        if crate::revision_compare::answers(&op) {
            self.compare_reply(root, &op, result);
            return;
        }
        if let workspace_ops::Op::Branch(branch) = &op {
            self.branch_op_finished(root, branch, &result);
        }
        // A list reply for a repository the panel no longer shows is stale.
        // Compare with the root new jobs run in (Perforce uses its command root).
        let stale = self.op_root().as_deref() != Some(root);
        match result {
            Ok(
                workspace_ops::Report::Branches(_)
                | workspace_ops::Report::Shelves(_)
                | workspace_ops::Report::Log(_)
                | workspace_ops::Report::Compare(_),
            ) if stale => {}
            // Only commit jobs report progress, and only Compare queries
            // answer with targets, revisions, and history: both go above.
            Ok(
                workspace_ops::Report::Pushing
                | workspace_ops::Report::CompareTargets(_)
                | workspace_ops::Report::Revision(_)
                | workspace_ops::Report::History(_),
            ) => {}
            Ok(workspace_ops::Report::Message(text)) => self.workspace_op_succeeded(&op, text),
            Ok(workspace_ops::Report::Branches(names)) => self.git.branches = names,
            // Amend edits the last message; only fill a draft the
            // user has not started typing over.
            Ok(workspace_ops::Report::HeadMessage(text)) => {
                if self.git.amend && self.git.commit.trim().is_empty() {
                    self.git.commit = text;
                }
            }
            Ok(workspace_ops::Report::Shelves(list)) => self.git.shelves = list,
            Ok(workspace_ops::Report::Log(rows)) => self.git.log = rows,
            Ok(workspace_ops::Report::Compare(data)) => {
                self.git.compare_root.clone_from(&data.root);
                self.git.compare = Some(data);
            }
            Err(error) => {
                if matches!(
                    op,
                    workspace_ops::Op::Perforce(_) | workspace_ops::Op::PerforceRun(_)
                ) {
                    self.refresh_request = None;
                }
                if let workspace_ops::Op::Rename { from, to } = &op {
                    self.explorer_undoing
                        .retain(|pair| pair != &(from.clone(), to.clone()));
                }
                self.workspace_op_failed(op, error, stale);
            }
        }
    }

    /// A long Perforce or sync run replied: its slot ends. True when Cancel
    /// stopped it: a notice, not a failure.
    fn long_run_cancelled(
        &mut self,
        root: &std::path::Path,
        op: &workspace_ops::Op,
        result: &Result<workspace_ops::Report, String>,
    ) -> bool {
        match op {
            workspace_ops::Op::PerforceRun(_) => {
                self.git.perforce.finish();
                let cancelled = crate::perforce::RUN_CANCELLED;
                if result.as_ref().is_err_and(|error| error == cancelled) {
                    // The workspace may still need a look.
                    self.refresh_request = None;
                    self.info = Some(cancelled.into());
                    return true;
                }
                false
            }
            workspace_ops::Op::SyncRun(run) => self.sync_finished(root, &run.op, result),
            _ => false,
        }
    }

    /// A failed job. Lists the panel loads by itself show their error in
    /// the panel and the log only; a new repository's `git log` is an
    /// empty History. Other failures go to the status bar with their
    /// operation name, and with Retry when the job can run again.
    fn workspace_op_failed(&mut self, op: workspace_ops::Op, error: String, stale: bool) {
        if let Some(list) = background_list(&op) {
            if stale {
                return;
            }
            if matches!(op, workspace_ops::Op::Log)
                && error.contains("does not have any commits yet")
            {
                self.git.log.clear();
                return;
            }
            let text = format!(
                "{list} failed: {}",
                crate::app::frame::status_line::first_line(&error)
            );
            self.error.log_error(&error);
            self.git.list_error = Some(text);
            return;
        }
        let shown = error.clone();
        self.report_status_error(error);
        if let Some(operation) = operation_name(&op) {
            self.error.attach_origin(
                &shown,
                crate::app::frame::status_line::ErrorOrigin {
                    operation,
                    retry: retryable(&op).then_some(op),
                },
            );
        }
    }

    fn workspace_op_succeeded(&mut self, op: &workspace_ops::Op, text: String) {
        // Open tabs and buffers follow a renamed or moved
        // file instead of editing a path that is gone.
        let moved: Vec<(PathBuf, PathBuf)> = match op {
            workspace_ops::Op::Rename { from, to } => vec![(from.clone(), to.clone())],
            workspace_ops::Op::Move { from, to_dir } => from
                .iter()
                .filter_map(|source| Some((source.clone(), to_dir.join(source.file_name()?))))
                .collect(),
            _ => Vec::new(),
        };
        for (from, to) in &moved {
            self.remap_moved_path(from, to);
        }
        self.explorer_record_move(moved);
        self.info = Some(text);
        self.refresh_request = None;
        if matches!(
            op,
            workspace_ops::Op::Switch(_)
                | workspace_ops::Op::Shelve(_)
                | workspace_ops::Op::Unshelve(_)
                | workspace_ops::Op::DropShelf(_)
                | workspace_ops::Op::Branch(_)
                | workspace_ops::Op::SyncRun(_)
        ) {
            self.git.list_root = None;
            // History and blame changed; open views reload.
            self.git_views.clear();
        }
    }

    pub(super) fn handle_git_view(&mut self, key: &str, view: GitView) {
        let view = match view {
            GitView::Log(log) => match self.log_answer(key, log) {
                Some(view) => view,
                None => return,
            },
            view => view,
        };
        // A view closed while loading leaves no entry to fill.
        if let Some(slot) = self.git_views.get_mut(key) {
            *slot = view;
        }
    }

    pub(super) fn handle_search(&mut self, id: u64, result: Result<Vec<search::Hit>, String>) {
        if id != self.explorer_search_generation {
            return;
        }
        self.explorer_search_pending = false;
        match result {
            Ok(hits) => {
                self.explorer_search = hits;
                self.explorer_search_error = None;
            }
            Err(error) => {
                self.explorer_search.clear();
                self.explorer_search_error = Some(error);
            }
        }
    }
}

/// Lists the Git panel loads by itself: their failures stay in the panel.
fn background_list(op: &workspace_ops::Op) -> Option<&'static str> {
    use workspace_ops::Op;
    match op {
        Op::Branches => Some("Branches"),
        Op::Log => Some("History"),
        Op::Shelves => Some("Shelves"),
        Op::Compare { .. } => Some("Compare"),
        Op::HeadMessage => Some("Last commit message"),
        _ => None,
    }
}

/// The operation name a failure summary starts with.
fn operation_name(op: &workspace_ops::Op) -> Option<&'static str> {
    use workspace_ops::Op;
    Some(match op {
        Op::Stage(_) => "Stage",
        Op::StageAll(_) => "Stage all",
        Op::Unstage(_) => "Unstage",
        Op::Discard { .. } => "Discard",
        Op::Delete(_) => "Move to Trash",
        Op::Shelve(_) => "Shelve",
        Op::Unshelve(_) => "Unshelve",
        Op::DropShelf(_) => "Drop shelf",
        Op::Switch(_) => "Switch branch",
        Op::CreatePr => "Create PR",
        Op::CreateFile(_) => "New file",
        Op::CreateDir(_) => "New folder",
        Op::Duplicate(_) => "Duplicate",
        Op::Rename { .. } => "Rename",
        Op::Move { .. } => "Move",
        Op::Copy { .. } => "Copy",
        Op::Branch(op) => op.name(),
        Op::SyncRun(run) => run.op.name(),
        _ => return None,
    })
}

/// Jobs that a Retry click may run again as they are. Discard, delete,
/// file moves and stash writes are left out: a partial first try may
/// have changed what they act on.
fn retryable(op: &workspace_ops::Op) -> bool {
    use workspace_ops::Op;
    matches!(
        op,
        Op::Stage(_) | Op::StageAll(_) | Op::Unstage(_) | Op::Switch(_) | Op::CreatePr
    )
}
