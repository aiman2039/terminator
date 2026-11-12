//! Last turn jobs: the daemon's turn and file list, a diff of a file
//! against its turn-start snapshot, and the confirmed revert. The revert
//! checks every file first (snapshot readable, file still as listed) and
//! writes nothing when one check fails; then it reverts file by file
//! through `LocalHistoryRevert` and stops at the first failure.
use super::Services;
use crate::local_history::turn::{RevertItem, RevertReport, TurnOp, TurnReply};
use std::path::Path;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response, local_history};

impl Services {
    pub(super) async fn turn_job(&self, op: TurnOp, cancel: CancellationToken) -> TurnReply {
        match op {
            TurnOp::Load(session) => {
                let request = Request::LocalHistoryTurn {
                    session: session.clone(),
                };
                let result = match self.client().rpc(request).await {
                    Ok(Response::LocalHistoryTurn(turn)) => Ok(turn),
                    Ok(_) => Err("Unexpected Local History reply".into()),
                    Err(error) => Err(format!("{error:#}")),
                };
                TurnReply::Loaded(session, result)
            }
            TurnOp::Diff { path, base } => {
                let snapshot = match base {
                    Some(id) => self.local_history_text(&path, id).await,
                    None => Ok(String::new()),
                };
                let result = match snapshot {
                    Ok(text) => {
                        self.snapshot_diff((path.clone(), text), "Turn start", cancel)
                            .await
                    }
                    Err(error) => Err(error),
                };
                TurnReply::Diff(path, result)
            }
            TurnOp::Revert(session, items) => {
                TurnReply::Reverted(session, self.turn_revert(items, cancel).await)
            }
        }
    }

    async fn turn_revert(&self, items: Vec<RevertItem>, cancel: CancellationToken) -> RevertReport {
        let mut report = RevertReport::default();
        for item in &items {
            if let Err(error) = self.local_history_text(&item.path, item.base.clone()).await {
                let why = format!("its turn-start snapshot cannot be read ({error})");
                report.refused.push((item.path.clone(), why));
            }
        }
        let listed = items.clone();
        let changed = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(
                    listed
                        .iter()
                        .filter_map(|item| {
                            still_as_listed(&item.path, item.newest.as_deref())
                                .err()
                                .map(|why| (item.path.clone(), why))
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .await;
        match changed {
            Ok(changed) => report.refused.extend(changed),
            Err(error) => report
                .refused
                .push((Path::new("").to_path_buf(), format!("{error:#}"))),
        }
        if !report.refused.is_empty() {
            return report;
        }
        for (at, item) in items.iter().enumerate() {
            let request = Request::LocalHistoryRevert {
                path: item.path.clone(),
                id: item.base.clone(),
            };
            match self.client().rpc(request).await {
                Ok(_) => report.reverted.push(item.path.clone()),
                Err(error) => {
                    report.failed = Some((item.path.clone(), format!("{error:#}")));
                    report.not_tried = items
                        .iter()
                        .skip(at.saturating_add(1))
                        .map(|i| i.path.clone())
                        .collect();
                    break;
                }
            }
        }
        report
    }
}

/// Whether `path` still holds the content Local History saw last
/// (`newest`), or is gone. Anything else changed after the list loaded and
/// must not be overwritten unseen.
pub(super) fn still_as_listed(path: &Path, newest: Option<&str>) -> Result<(), String> {
    match std::fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot be read ({error})")),
        Ok(bytes) if newest == Some(local_history::hash(&bytes).as_str()) => Ok(()),
        Ok(_) => Err("changed after the list loaded; reload the page".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::still_as_listed;
    use terminator_core::local_history::hash;

    #[test]
    fn a_file_changed_after_the_list_loaded_is_not_reverted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "agent edit").unwrap();
        let listed = hash(b"agent edit");
        assert!(still_as_listed(&path, Some(&listed)).is_ok());
        std::fs::write(&path, "user typed after").unwrap();
        assert!(still_as_listed(&path, Some(&listed)).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(
            still_as_listed(&path, Some(&listed)).is_ok(),
            "a deleted file is restored"
        );
    }
}
