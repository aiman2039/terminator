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

type Done<T> = tokio::sync::mpsc::UnboundedReceiver<Result<T, String>>;

enum Pending {
    Load(Done<MergeDoc>),
    /// A save; `true` stages the file once it lands.
    Save(Done<()>, bool),
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
            self.error = Some("Open the merge tool from a Git project".into());
            return;
        };
        let tab = super::tab(root, path);
        let showing = self
            .selected
            .as_ref()
            .and_then(|project| self.layouts.get(project))
            .is_some_and(|workspace| workspace.contains(&tab));
        if !showing {
            self.merge.remove(&tab.key());
        }
        self.open_git_tab(tab);
    }

    /// Run blocking `work` on the file pool; the receiver gets its result.
    pub(crate) fn file_pool_job<T: Send + 'static>(
        &mut self,
        label: String,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Option<Done<T>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
        match submitted {
            Ok(_) => Some(rx),
            Err(error) => {
                self.error = Some(format!("Could not start the file operation: {error}"));
                None
            }
        }
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
                if let Ok(result) = rx.try_recv() {
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
            Some(Pending::Save(rx, stage)) => {
                let stage = *stage;
                if let Ok(result) = rx.try_recv() {
                    state.pending = None;
                    self.merge_saved(&key, path, (stage, result));
                }
            }
            None => {}
        }
    }

    /// Discard the tab's state and read the conflict stages again.
    pub(crate) fn reload_merge(&mut self, key: &str, root: &Path, path: &Path) {
        let (root, path) = (root.to_path_buf(), path.to_path_buf());
        let label = path.display().to_string();
        let rx = self.file_pool_job(label, move || load(&root, &path));
        self.merge.insert(
            key.to_owned(),
            MergeTab {
                pending: rx.map(Pending::Load),
                ..MergeTab::default()
            },
        );
    }

    /// Write the result; `stage` also marks the file resolved (`git add`)
    /// once the write lands. Only explicit clicks call this.
    pub(crate) fn save_merge(&mut self, key: &str, path: &Path, stage: bool) {
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
        if let (Some(state), Some(rx)) = (self.merge.get_mut(key), rx) {
            state.pending = Some(Pending::Save(rx, stage));
            state.message = None;
        }
    }

    fn merge_saved(&mut self, key: &str, path: &Path, (stage, result): (bool, Result<(), String>)) {
        let Some(state) = self.merge.get_mut(key) else {
            return;
        };
        match result {
            Ok(()) => {
                state.dirty = false;
                state.error = None;
                state.message = Some(if stage {
                    "Saved and marked resolved".into()
                } else {
                    "Saved".into()
                });
                if stage {
                    self.queue_workspace(workspace_ops::Op::Stage(path.to_path_buf()));
                }
            }
            Err(error) => state.error = Some(error),
        }
    }
}
