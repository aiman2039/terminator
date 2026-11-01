//! Local History jobs: daemon reads and the confirmed revert, plus the
//! diff build on the filesystem pool.
use super::Services;
use crate::local_history::{Op, Reply};
use crate::{Update, diff};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

impl Services {
    pub(super) async fn local_history_job(
        &self,
        op: Op,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let reply = match op {
            Op::List(path) => {
                let result = match self
                    .client()
                    .rpc(Request::LocalHistoryList { path: path.clone() })
                    .await
                {
                    Ok(Response::LocalHistory(entries)) => Ok(entries),
                    Ok(_) => Err("Unexpected Local History reply".into()),
                    Err(error) => Err(format!("{error:#}")),
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
                let current = std::fs::read_to_string(&path).unwrap_or_default();
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
