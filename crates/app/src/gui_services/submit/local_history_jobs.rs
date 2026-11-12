//! Local History jobs: daemon reads and the confirmed revert, plus the
//! diff build on the filesystem pool.
use super::Services;
use crate::local_history::{Op, Reply};
use crate::{Update, diff};
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
use terminator_core::async_service::CancellationToken;
use terminator_core::local_history::{self, Entry};
use terminator_core::{Request, Response};

/// Entries asked for per `LocalHistoryPage`.
const PAGE: u64 = 500;

impl Services {
    pub(super) async fn local_history_job(
        &self,
        op: Op,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let reply = match op {
            Op::List(path) => {
                let result = if self.advertises(local_history::LIMITS_CAPABILITY) {
                    self.local_history_pages(&path).await
                } else {
                    match self
                        .client()
                        .rpc(Request::LocalHistoryList { path: path.clone() })
                        .await
                    {
                        Ok(Response::LocalHistory(entries)) => Ok(entries),
                        Ok(_) => Err("Unexpected Local History reply".into()),
                        Err(error) => Err(format!("{error:#}")),
                    }
                };
                Reply::Listed(path, result)
            }
            Op::Diff { path, id } => {
                let result = self
                    .local_history_diff(path.clone(), id.clone(), cancel)
                    .await;
                Reply::Diff(path, id, result)
            }
            Op::Revert { path, id } => {
                let result = self
                    .client()
                    .rpc(Request::LocalHistoryRevert {
                        path: path.clone(),
                        id,
                    })
                    .await
                    .map(|_| ())
                    .map_err(|error| format!("{error:#}"));
                Reply::Reverted(path, result)
            }
        };
        updates.push(Update::LocalHistory(reply));
        Ok(())
    }

    /// The whole timeline in pages, so no reply exceeds the IPC frame.
    async fn local_history_pages(&self, path: &std::path::Path) -> Result<Vec<Entry>, String> {
        let mut entries = Vec::new();
        loop {
            let request = Request::LocalHistoryPage {
                path: path.to_path_buf(),
                offset: u64::try_from(entries.len()).unwrap_or(u64::MAX),
                limit: PAGE,
            };
            let (page, total) = match self.client().rpc(request).await {
                Ok(Response::LocalHistoryPage { entries, total }) => (entries, total),
                Ok(_) => return Err("Unexpected Local History reply".into()),
                Err(error) => return Err(format!("{error:#}")),
            };
            let done = page.is_empty();
            entries.extend(page);
            if done || u64::try_from(entries.len()).unwrap_or(u64::MAX) >= total {
                return Ok(entries);
            }
        }
    }

    async fn local_history_diff(
        &self,
        path: PathBuf,
        id: String,
        cancel: CancellationToken,
    ) -> Result<Arc<diff::DiffDocument>, String> {
        let snapshot = match self
            .client()
            .rpc(Request::LocalHistoryRead {
                path: path.clone(),
                id,
            })
            .await
        {
            Ok(Response::Text(text)) => text,
            Ok(_) => return Err("Unexpected Local History reply".into()),
            Err(error) => return Err(format!("{error:#}")),
        };
        self.fs()
            .run(&cancel, move || {
                let current = read_current(&path)
                    .with_context(|| format!("Could not read {}", path.display()))?;
                let name = path
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.clone());
                let mut doc = diff::build_with_context(diff::BuildWithContextInput {
                    path: &name,
                    left: &snapshot,
                    right: &current,
                    staged: false,
                    context: 3,
                });
                doc.read_only = true;
                doc.left_label = "Snapshot".into();
                doc.right_label = "On disk".into();
                anyhow::Ok(Arc::new(doc))
            })
            .await
            .map_err(|error| format!("{error:#}"))
    }
}

/// Current file text for a Local History diff. A deleted file diffs as empty;
/// any other read failure is reported instead of showing a false deletion.
fn read_current(path: &std::path::Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::read_current;

    #[test]
    fn deleted_file_diffs_as_empty_but_unreadable_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_current(&dir.path().join("gone.txt")).unwrap(), "");
        assert!(read_current(dir.path()).is_err());
        std::fs::write(dir.path().join("a.txt"), "text").unwrap();
        assert_eq!(read_current(&dir.path().join("a.txt")).unwrap(), "text");
    }
}
