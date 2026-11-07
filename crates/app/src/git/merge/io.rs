//! Merge tab state and its background reads and writes.
use super::{MergeDoc, Side};
use crate::{App, Tab, workspace_ops};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// Merge tabs by tab key.
pub(crate) type MergeTabs = HashMap<String, MergeTab>;

pub(crate) type Done<T> = tokio::sync::mpsc::UnboundedReceiver<Result<T, String>>;

/// The result of a [`App::file_pool_job`], once there is one. A job that
/// ended without sending (timed out, panicked, cancelled) is an error, so
/// the view waiting for it stops waiting.
pub(crate) fn take_done<T>(rx: &mut Done<T>) -> Option<Result<T, String>> {
    use tokio::sync::mpsc::error::TryRecvError;
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            Some(Err("The file operation stopped before it finished".into()))
        }
    }
}

enum Pending {
    Load(Done<MergeDoc>),
    /// A save of the file at the path; `true` stages it once it lands.
    Save(Done<()>, bool, PathBuf),
}

/// One merge tab: the document being resolved and its pending I/O.
#[derive(Default)]
pub(crate) struct MergeTab {
    pub doc: Option<MergeDoc>,
    pub error: Option<String>,
    pub message: Option<String>,
    /// The result differs from the file on disk.
    pub dirty: bool,
    /// Chunk the navigation last moved to.
    pub current: Option<usize>,
    /// Chunk to scroll into view next frame.
    pub scroll_to: Option<usize>,
    /// Syntax spans of hand-edited chunk text, by text hash.
    pub highlights: HashMap<u64, std::sync::Arc<Vec<Vec<crate::diff::DiffSpan>>>>,
    pending: Option<Pending>,
}

impl MergeTab {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// A write is in flight; its result (and `git add`) must still land.
    pub(crate) fn saving(&self) -> bool {
        matches!(self.pending, Some(Pending::Save(..)))
    }

    /// A tab whose save is still running (the sender is returned so the
    /// receiver stays open).
    #[cfg(test)]
    pub(crate) fn saving_for_test() -> (Self, tokio::sync::mpsc::UnboundedSender<Result<(), String>>)
    {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let tab = Self {
            pending: Some(Pending::Save(rx, true, PathBuf::from("/a/conflict.rs"))),
            ..Self::default()
        };
        (tab, tx)
    }
}

/// Read the conflict stages of `path` and build the merge (blocking).
pub(crate) fn load(root: &Path, path: &Path) -> Result<MergeDoc, String> {
    let relative = terminator_git::relative_path(root, path).map_err(|e| e.to_string())?;
    let options = terminator_core::CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let stages = terminator_git::conflict_stages(root, &relative, &options)
        .map_err(|error| error.to_string())?;
    let side = |bytes: Option<Vec<u8>>| -> Result<Side, String> {
        let present = bytes.is_some();
        let bytes = bytes.unwrap_or_default();
        if !terminator_git::check_text(&bytes) {
            return Err(
                "Binary conflicts cannot be merged here; resolve them in a terminal".into(),
            );
        }
        let mut side = Side::new(String::from_utf8(bytes).unwrap_or_default(), present);
        side.spans = crate::diff::highlight_spans(&relative, &side.text);
        Ok(side)
    };
    Ok(MergeDoc::new(
        side(stages.base)?,
        side(stages.ours)?,
        side(stages.theirs)?,
    ))
}

impl App {
    /// Open (or focus) the merge tab for a conflicted file. Explicit
    /// opens of a tab that is not showing start from a fresh read.
    pub(crate) fn open_merge(&mut self, path: PathBuf) {
        let Some(root) = self.context.as_ref().and_then(|c| c.root.clone()) else {
            self.error.set("Open the merge tool from a Git project");
            return;
        };
        let tab = super::tab(root, path);
        let showing = self
            .selected
            .as_ref()
            .and_then(|project| self.layouts.get(project))
            .is_some_and(|workspace| workspace.contains(&tab));
        // A pending save must still land (and stage) after the tab is left.
        let saving = self
            .merge
            .get(&tab.key())
            .is_some_and(|state| matches!(state.pending, Some(Pending::Save(..))));
        if !showing && !saving {
            self.merge.remove(&tab.key());
        }
        self.open_git_tab(tab);
    }

    /// Run blocking `work` on the file pool; the receiver gets its result.
    /// A refused job answers at once with the error, so its view never
    /// waits for a job that was not sent (read it with [`take_done`]).
    pub(crate) fn file_pool_job<T: Send + 'static>(
        &mut self,
        label: String,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Done<T> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let refused = tx.clone();
        let service = self.services.clone();
        let submitted = self.services.handle().submit(
            OperationContext::new("merge-file", label, Policy::OrderedMutation),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || anyhow::Ok(work()))
                    .await
                    .unwrap_or_else(|error| Err(format!("{error:#}")));
                let _ = tx.send(result);
                Ok(Vec::new())
            },
        );
        if let Err(error) = submitted {
            let _ = refused.send(Err(format!("Could not start the file operation: {error}")));
        }
        rx
    }

    /// Start the first read of a merge tab, then take finished I/O.
    pub(crate) fn poll_merge(&mut self, tab: &Tab) {
        let Tab::Merge { cwd, path } = tab else {
            return;
        };
        let key = tab.key();
        if !self.merge.contains_key(&key) {
            self.reload_merge(&key, cwd, path);
        }
        let Some(state) = self.merge.get_mut(&key) else {
            return;
        };
        match state.pending.as_mut() {
            Some(Pending::Load(rx)) => {
                if let Some(result) = take_done(rx) {
                    state.pending = None;
                    match result {
                        Ok(doc) => {
                            state.current = doc.neighbor(None, true);
                            state.scroll_to = state.current;
                            state.doc = Some(doc);
                            state.dirty = false;
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
            }
            Some(Pending::Save(..)) | None => {}
        }
        self.poll_merge_saves();
    }

    /// Take finished merge saves of every tab, drawn or not, so a
    /// Mark resolved still stages and errors still show after the tab
    /// is left.
    pub(crate) fn poll_merge_saves(&mut self) {
        let mut done = Vec::new();
        for (key, state) in &mut self.merge {
            if let Some(Pending::Save(rx, stage, path)) = state.pending.as_mut() {
                let Some(result) = take_done(rx) else {
                    continue;
                };
                done.push((key.clone(), path.clone(), *stage, result));
                state.pending = None;
            }
        }
        for (key, path, stage, result) in done {
            if let Err(error) = &result {
                self.error.set(error.clone());
            }
            self.merge_saved(&key, &path, (stage, result));
        }
    }

    /// Discard the tab's state and read the conflict stages again.
    /// Ignored while a save runs: replacing the tab would drop the save's
    /// result, and the read could race the write.
    pub(crate) fn reload_merge(&mut self, key: &str, root: &Path, path: &Path) {
        if self.merge.get(key).is_some_and(MergeTab::saving) {
            return;
        }
        let (root, path) = (root.to_path_buf(), path.to_path_buf());
        let label = path.display().to_string();
        let rx = self.file_pool_job(label, move || load(&root, &path));
        self.merge.insert(
            key.to_owned(),
            MergeTab {
                pending: Some(Pending::Load(rx)),
                ..MergeTab::default()
            },
        );
    }

    /// Write the result; `stage` also marks the file resolved (`git add`)
    /// once the write lands. Only explicit clicks call this. A commit
    /// running where the stage would go refuses Mark resolved.
    pub(crate) fn save_merge(&mut self, key: &str, path: &Path, stage: bool) {
        let repository = self.git_root();
        let risk = workspace_ops::CommitRisk::Index;
        if stage && self.refuse_during_commit(repository.as_deref(), risk) {
            return;
        }
        let Some(text) = self
            .merge
            .get(key)
            .and_then(|state| state.doc.as_ref())
            .map(MergeDoc::result_text)
        else {
            return;
        };
        let writing = path.to_path_buf();
        let rx = self.file_pool_job(path.display().to_string(), move || {
            std::fs::write(&writing, text)
                .map_err(|error| format!("Write {}: {error}", writing.display()))
        });
        if let Some(state) = self.merge.get_mut(key) {
            state.pending = Some(Pending::Save(rx, stage, path.to_path_buf()));
            state.message = None;
        }
    }

    /// A save landed. Its stage goes out unless a commit started since the
    /// click; then the file is saved but not marked resolved.
    fn merge_saved(&mut self, key: &str, path: &Path, (stage, result): (bool, Result<(), String>)) {
        if !self.merge.contains_key(key) {
            return;
        }
        let staged = result.is_ok()
            && stage
            && self.queue_workspace(workspace_ops::Op::Stage(path.to_path_buf()));
        let Some(state) = self.merge.get_mut(key) else {
            return;
        };
        match result {
            Ok(()) => {
                state.dirty = false;
                state.error = None;
                state.message = Some(if staged {
                    "Saved and marked resolved".into()
                } else {
                    "Saved".into()
                });
            }
            Err(error) => state.error = Some(error),
        }
    }
}
