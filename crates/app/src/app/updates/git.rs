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
                review.confirm_discard = None;
                review.selected = document
                    .hunks
                    .len()
                    .checked_sub(1)
                    .map(|last| review.selected.unwrap_or(0).min(last));
                self.diffs.insert(key, Ok(std::sync::Arc::new(document)));
            }
            Err(error) if self.diffs.get(&key).is_some_and(Result::is_ok) => {
                self.error = Some(format!(
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
        review.busy = false;
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
        review.busy = false;
        review.confirm_discard = None;
        match result {
            Ok(outcome) => {
                if review.saving_edits {
                    review.buffer = None;
                }
                if undoing {
                    review.undo.pop();
                }
                if let Some(undo) = outcome.undo {
                    if review.undo.len() == DIFF_UNDO_LIMIT {
                        review.undo.remove(0);
                    }
                    review.undo.push(undo);
                }
                review.message = Some(outcome.message);
            }
            Err(error) => review.message = Some(error),
        }
        review.saving_edits = false;
        if let Tab::Diff { cwd, path, .. } = tab {
            self.reload_open_diffs(&cwd, &path);
            self.refresh_request = None;
        }
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
        op: workspace_ops::Op,
        result: Result<workspace_ops::Report, String>,
    ) {
        match result {
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
                if matches!(op, workspace_ops::Op::Perforce(_)) {
                    self.refresh_request = None;
                }
                if matches!(op, workspace_ops::Op::Commit { .. }) {
                    self.restore_failed_commit(&error);
                }
                self.report_status_error(error);
            }
        }
    }

    fn workspace_op_succeeded(&mut self, op: &workspace_ops::Op, text: String) {
        // Open tabs and buffers follow a renamed or moved
        // file instead of editing a path that is gone.
        match op {
            workspace_ops::Op::Rename { from, to } => {
                self.remap_moved_path(from, to);
            }
            workspace_ops::Op::Move { from, to_dir } => {
                for source in from {
                    if let Some(name) = source.file_name() {
                        self.remap_moved_path(source, &to_dir.join(name));
                    }
                }
            }
            _ => {}
        }
        if matches!(op, workspace_ops::Op::Commit { .. }) {
            self.git.commit_inflight = None;
        }
        self.info = Some(text);
        self.refresh_request = None;
        if matches!(
            op,
            workspace_ops::Op::Commit { .. }
                | workspace_ops::Op::Switch(_)
                | workspace_ops::Op::Shelve(_)
                | workspace_ops::Op::Unshelve(_)
                | workspace_ops::Op::DropShelf(_)
        ) {
            self.git.list_root = None;
            // History and blame changed; open views reload.
            self.git_views.clear();
        }
    }

    pub(super) fn handle_git_view(&mut self, key: &str, view: GitView) {
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
