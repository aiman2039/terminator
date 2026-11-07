//! Local History page: one file's daemon-recorded snapshots, a native diff
//! of the selected snapshot against the file on disk, and a confirmed
//! Revert. It fills the main pane like Settings; it is not a layout tab.
use crate::{App, Job, diff::DiffDocument};
use std::{path::PathBuf, sync::Arc};
use terminator_core::async_service::Policy;
use terminator_core::local_history::{CAPABILITY, Entry};

/// Daemon work for the page; runs on the GUI services, never the UI thread.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    List(PathBuf),
    /// Diff snapshot `id` against the current file.
    Diff {
        path: PathBuf,
        id: String,
    },
    Revert {
        path: PathBuf,
        id: String,
    },
}

pub(crate) enum Reply {
    Listed(PathBuf, Result<Vec<Entry>, String>),
    Diff(PathBuf, String, Result<Arc<DiffDocument>, String>),
    Reverted(PathBuf, Result<(), String>),
}

#[derive(Default)]
pub(crate) struct Page {
    pub(crate) path: PathBuf,
    pub(crate) entries: Option<Result<Vec<Entry>, String>>,
    pub(crate) selected: Option<String>,
    pub(crate) doc: Option<Result<Arc<DiffDocument>, String>>,
    pub(crate) confirm_revert: bool,
    pub(crate) busy: bool,
}

impl Op {
    pub(crate) fn path(&self) -> &PathBuf {
        match self {
            Self::List(path) | Self::Diff { path, .. } | Self::Revert { path, .. } => path,
        }
    }

    /// The reply when the job could not run.
    pub(crate) fn failed(&self, error: String) -> Reply {
        match self {
            Self::List(path) => Reply::Listed(path.clone(), Err(error)),
            Self::Diff { path, id } => Reply::Diff(path.clone(), id.clone(), Err(error)),
            Self::Revert { path, .. } => Reply::Reverted(path.clone(), Err(error)),
        }
    }
}

/// Service route: reads replace each other; a revert is ordered.
pub(crate) fn route(op: &Op) -> (&'static str, String, Policy) {
    let path = op.path().display().to_string();
    match op {
        Op::List(_) => (
            "local-history",
            format!("list:{path}"),
            Policy::ReplaceableRead,
        ),
        Op::Diff { .. } => (
            "local-history",
            format!("diff:{path}"),
            Policy::ReplaceableRead,
        ),
        Op::Revert { .. } => (
            "local-history",
            format!("revert:{path}"),
            Policy::OrderedMutation,
        ),
    }
}

/// Whether the connected daemon records Local History.
pub(crate) fn supported(capabilities: &[String]) -> bool {
    capabilities.iter().any(|c| c == CAPABILITY)
}

impl App {
    pub(crate) fn local_history_supported(&self) -> bool {
        supported(&self.state.capabilities)
    }

    /// Opens the Local History page for `path` in the main pane. Does
    /// nothing visible but an error when the daemon lacks the capability.
    pub(crate) fn open_local_history(&mut self, path: PathBuf) {
        if !self.local_history_supported() {
            self.error = Some("Local History needs an updated session service.".into());
            return;
        }
        self.show_center(crate::CenterPage::LocalHistory);
        let _ = self.jobs.send(Job::LocalHistory(Op::List(path.clone())));
        self.local_history = Some(Page {
            path,
            ..Page::default()
        });
    }

    pub(crate) fn handle_local_history(&mut self, reply: Reply) {
        let Some(page) = self.local_history.as_mut() else {
            return;
        };
        match reply {
            Reply::Listed(path, result) if path == page.path => {
                let first = result
                    .as_ref()
                    .ok()
                    .and_then(|entries| entries.iter().find(|e| !e.id.is_empty()))
                    .map(|e| e.id.clone());
                page.entries = Some(result);
                page.busy = false;
                let keep = page.selected.clone().or(first);
                page.selected = None;
                if let Some(id) = keep {
                    self.select_local_history(id);
                }
            }
            Reply::Diff(path, id, result)
                if path == page.path && page.selected.as_deref() == Some(id.as_str()) =>
            {
                page.doc = Some(result);
            }
            Reply::Reverted(path, result) if path == page.path => {
                page.busy = false;
                match result {
                    Ok(()) => {
                        self.info = Some(format!("Reverted {}", path.display()));
                        let _ = self.jobs.send(Job::LocalHistory(Op::List(path)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            _ => {}
        }
    }

    pub(crate) fn select_local_history(&mut self, id: String) {
        let Some(page) = self.local_history.as_mut() else {
            return;
        };
        if page.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        page.selected = Some(id.clone());
        page.doc = None;
        page.confirm_revert = false;
        let _ = self.jobs.send(Job::LocalHistory(Op::Diff {
            path: page.path.clone(),
            id,
        }));
    }

    /// Sends the revert only after the user confirmed it on the page.
    pub(crate) fn revert_local_history(&mut self) {
        let Some(page) = self.local_history.as_mut() else {
            return;
        };
        let Some(id) = page.selected.clone() else {
            return;
        };
        page.confirm_revert = false;
        page.busy = true;
        let _ = self.jobs.send(Job::LocalHistory(Op::Revert {
            path: page.path.clone(),
            id,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_hides_local_history_when_the_daemon_lacks_the_capability() {
        assert!(!supported(&["worktrees-v1".into()]));
        assert!(supported(&[CAPABILITY.into()]));
    }
}
