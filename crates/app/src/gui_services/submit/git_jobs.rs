//! Git and diff job dispatch.
use super::{MISROUTED, Services};
use crate::gui_services::jobs::{DiffActionInput, DiffContextInput, DiffExpandInput};
use crate::{Job, Update};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;

impl Services {
    pub(super) async fn git_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::DiffAction(tab, request) => {
                self.diff_action(DiffActionInput {
                    tab,
                    request,
                    cancel,
                    updates,
                })
                .await
            }
            Job::DiffContext(tab, view) => {
                self.diff_context(DiffContextInput {
                    tab,
                    view,
                    cancel,
                    updates,
                })
                .await
            }
            Job::DiffExpand(tab, doc, gap, part) => {
                self.diff_expand(DiffExpandInput {
                    tab,
                    doc,
                    gap,
                    part,
                    cancel,
                    updates,
                })
                .await
            }
            Job::Diff(tab) => self.diff(tab, cancel, updates).await,
            Job::CreateWorktree(draft) => self.create_worktree(draft, cancel, updates).await,
            Job::CommitFiles(cwd, hash) => {
                self.commit_files(CommitFilesInput {
                    cwd,
                    hash,
                    cancel,
                    updates,
                })
                .await
            }
            Job::EditorBlame(path, revision, text) => {
                self.editor_blame(EditorBlameInput {
                    path,
                    revision,
                    text,
                    cancel,
                    updates,
                })
                .await
            }
            Job::GitBase(path, generation) => {
                let base = self.git_base_text(&path).await;
                updates.push(Update::GitBase(path, generation, base));
                Ok(())
            }
            Job::GitView(tab) => self.git_view(tab, cancel, updates).await,
            _ => anyhow::bail!(MISROUTED),
        }
    }

    async fn commit_files(&self, input: CommitFilesInput<'_>) -> Result<()> {
        let CommitFilesInput {
            cwd,
            hash,
            cancel,
            updates,
        } = input;
        let query = hash.clone();
        let files = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(crate::git_log::commit_files(&cwd, &query))
            })
            .await
            .unwrap_or_else(|error| Err(format!("{error:#}")));
        updates.push(Update::CommitFiles(hash, files));
        Ok(())
    }

    async fn editor_blame(&self, input: EditorBlameInput<'_>) -> Result<()> {
        let EditorBlameInput {
            path,
            revision,
            text,
            cancel,
            updates,
        } = input;
        let file = path.clone();
        let entries = self
            .fs()
            .run(&cancel, move || {
                let cwd = file
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                anyhow::Ok(
                    crate::git_log::blame_with_contents(&cwd, &file, Some(&text))
                        .map(|blame| blame.entries),
                )
            })
            .await
            .unwrap_or_else(|error| Err(format!("{error:#}")));
        updates.push(Update::EditorBlame(path, revision, entries));
        Ok(())
    }
}

struct CommitFilesInput<'a> {
    cwd: PathBuf,
    hash: String,
    cancel: CancellationToken,
    updates: &'a mut Vec<Update>,
}

/// A native buffer to blame at its buffer revision.
struct EditorBlameInput<'a> {
    path: PathBuf,
    revision: u64,
    text: String,
    cancel: CancellationToken,
    updates: &'a mut Vec<Update>,
}
