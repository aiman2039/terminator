use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use terminator_core::{local_history, *};

use super::super::super::shared::{Shared, relock};
use super::Outcome;
use crate::workers::{LocalHistoryJob, LocalHistoryWorker};

impl Shared {
    pub(super) fn on_local_history_list(&self, path: &Path) -> Result<Outcome> {
        ensure!(path.is_absolute(), "Local History needs an absolute path");
        let entries = LocalHistoryWorker::store(self).list(path)?;
        Ok(Outcome::Reply(Response::LocalHistory(entries)))
    }

    /// One page of the timeline, so a large one fits the IPC frame.
    pub(super) fn on_local_history_page(
        &self,
        path: &Path,
        offset: u64,
        limit: u64,
    ) -> Result<Outcome> {
        ensure!(path.is_absolute(), "Local History needs an absolute path");
        let (entries, total) = LocalHistoryWorker::store(self).list_page(
            path,
            usize::try_from(offset).unwrap_or(usize::MAX),
            usize::try_from(limit).unwrap_or(usize::MAX),
        )?;
        Ok(Outcome::Reply(Response::LocalHistoryPage {
            entries,
            total: u64::try_from(total).unwrap_or(u64::MAX),
        }))
    }

    pub(super) fn on_local_history_read(&self, path: &Path, id: &str) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::Text(
            self.snapshot_text(path, id)?,
        )))
    }

    pub(super) fn on_local_history_label(&self, project: &str, label: String) -> Result<Outcome> {
        ensure!(
            !label.trim().is_empty() && label.len() <= 200,
            "A label needs 1–200 characters"
        );
        let root = relock(&self.state)
            .projects
            .iter()
            .find(|p| p.id == project)
            .context("Unknown project")?
            .path
            .clone();
        self.queue_local_history_label(root, label);
        Ok(Outcome::Reply(Response::Ok))
    }

    /// Saves the current content as "Before revert", then writes snapshot
    /// `id` over `path`. Only an explicit, confirmed GUI action sends this.
    pub(super) fn on_local_history_revert(&self, path: &Path, id: &str) -> Result<Outcome> {
        let text = self.snapshot_text(path, id)?;
        revert_file(&LocalHistoryWorker::store(self), path, &text)?;
        Ok(Outcome::Reply(Response::Ok))
    }

    /// The text of snapshot `id`, which must belong to `path`'s timeline.
    fn snapshot_text(&self, path: &Path, id: &str) -> Result<String> {
        ensure!(path.is_absolute(), "Local History needs an absolute path");
        let store = LocalHistoryWorker::store(self);
        ensure!(
            !id.is_empty() && store.list(path)?.iter().any(|entry| entry.id == id),
            "No such Local History snapshot for {}",
            path.display()
        );
        Ok(store.read(id)?)
    }

    /// Never blocks: a full queue drops the label.
    pub(crate) fn queue_local_history_label(&self, root: PathBuf, label: String) {
        let _ = self
            .local_history
            .try_send(LocalHistoryJob::Label { root, label });
    }

    /// Marks "Before agent run" in the project's timeline when a hook moves
    /// an agent into `Running`. Hook lifecycle only; never terminal text.
    pub(super) fn label_agent_run(&self, event: &HookEvent) {
        if event.state != AgentState::Running {
            return;
        }
        let root = {
            let state = relock(&self.state);
            let already_running = state
                .agents
                .iter()
                .any(|a| a.invocation_id == event.agent_invocation_id && a.state == event.state);
            let session = state
                .sessions
                .iter()
                .find(|s| s.id == event.terminal_session_id && s.remote.is_none());
            match session {
                Some(session) if !already_running => state
                    .projects
                    .iter()
                    .find(|p| p.id == session.project_id)
                    .map(|p| p.path.clone()),
                _ => None,
            }
        };
        if let Some(root) = root {
            self.queue_local_history_label(root, local_history::BEFORE_AGENT_RUN.into());
        }
    }
}

/// Keeps the current content as "Before revert", then writes `text` over
/// `path`. Content Local History cannot keep (too large, binary) is never
/// overwritten: the revert is refused instead.
fn revert_file(store: &local_history::Store, path: &Path, text: &str) -> Result<()> {
    if let Ok(current) = std::fs::read(path) {
        ensure!(
            local_history::recordable(&current),
            "The current content of {} cannot be kept in Local History (too large or not text); revert refused",
            path.display()
        );
        store.record(local_history::Record {
            path,
            bytes: &current,
            source: local_history::Source::External,
            label: Some(local_history::BEFORE_REVERT),
            now_ms: local_history::now_ms(),
        })?;
    }
    // Rename a complete temp file into place so a crash never leaves a
    // half-written file; a symlink keeps pointing at the reverted file.
    atomic_replace(path, text.as_bytes()).with_context(|| format!("Writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_refuses_to_overwrite_content_it_cannot_keep() {
        let data = tempfile::tempdir().unwrap();
        let store = local_history::Store::in_data(data.path());
        let path = data.path().join("file.bin");
        std::fs::write(&path, b"binary\0content").unwrap();
        assert!(revert_file(&store, &path, "old text").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"binary\0content");
        std::fs::write(&path, "new text").unwrap();
        revert_file(&store, &path, "old text").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old text");
        assert_eq!(store.list(&path).unwrap().len(), 1, "before-revert kept");
    }

    #[test]
    fn local_history_page_reply_fits_one_frame_for_a_large_timeline() {
        let data = tempfile::tempdir().unwrap();
        let store = local_history::Store::in_data(data.path());
        let path = data.path().join("big.log");
        for n in 0..1500u64 {
            store
                .record(local_history::Record {
                    path: &path,
                    bytes: n.to_string().as_bytes(),
                    source: local_history::Source::External,
                    label: None,
                    now_ms: n,
                })
                .unwrap();
        }
        let (page, total) = store.list_page(&path, 0, usize::MAX).unwrap();
        assert_eq!(total, 1500);
        assert_eq!(page.len(), local_history::MAX_PAGE, "a page is capped");
    }
}
