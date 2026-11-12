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
            Job::WorktreeProbe(path, draft) => {
                let git = self.is_git_checkout(path, &cancel).await;
                updates.push(Update::WorktreeProbe(draft, git));
                Ok(())
            }
            job @ (Job::WorktreeStats(..) | Job::TaskChanges(..)) => {
                updates.extend(self.task_worktree_job(job, &cancel).await);
                Ok(())
            }
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
                updates.push(self.git_base(path, generation, &cancel).await);
                Ok(())
            }
            job @ Job::StageGutterBlock { .. } => {
                self.stage_gutter_block(job, &cancel, updates).await;
                Ok(())
            }
            Job::GitView(tab) => self.git_view(tab, cancel, updates).await,
            Job::GitLog(tab, query) => self.git_log((tab, query), cancel, updates).await,
            _ => anyhow::bail!(MISROUTED),
        }
    }

    /// A worktree row's stats, or the files a task worktree changed since
    /// its base (on the file pool). Read-only Git.
    async fn task_worktree_job(&self, job: Job, cancel: &CancellationToken) -> Option<Update> {
        let (path, base) = match job {
            Job::WorktreeStats(path, base) => {
                let git = self.worktree_git(&path, base).await;
                return Some(Update::WorktreeStats(path, git));
            }
            Job::TaskChanges(path, base) => (path, base),
            _ => return None,
        };
        let (worktree, revision) = (path.clone(), base.clone());
        let files = self
            .fs()
            .run(cancel, move || {
                anyhow::Ok(crate::task_changes::list_changes(&worktree, &revision))
            })
            .await
            .unwrap_or_else(|error| Err(format!("{error:#}")));
        Some(Update::TaskChanges((path, base), files))
    }

    /// The `HEAD` text of `path` for gutter markers, and the root of its
    /// repository: the gutter Stage matches that root with a running commit.
    async fn git_base(&self, path: PathBuf, generation: u64, cancel: &CancellationToken) -> Update {
        let base = self.git_base_text(&path).await;
        let root = match base {
            Some(_) => self.git_toplevel(&path, cancel).await,
            None => None,
        };
        Update::GitBase(path, generation, base, root)
    }

    /// The worktree's branch and `+added −deleted` from its task base to
    /// the working tree: the saved base, else the merge base with the main
    /// checkout's branch, else HEAD (uncommitted changes only). Read-only
    /// Git (no optional locks); `None` when Git fails.
    async fn worktree_git(
        &self,
        path: &std::path::Path,
        saved: Option<String>,
    ) -> Option<crate::worktree_stats::WorktreeGit> {
        let run = |args: &[&str]| {
            terminator_git::run(
                self.processes(),
                self.fs(),
                terminator_git::RunInput {
                    cwd: path,
                    args: args.iter().map(Into::into).collect(),
                    options: terminator_core::CommandOptions::default(),
                },
            )
        };
        let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).trim().to_owned();
        let (base, base_name) = if let Some(rev) = saved.filter(|rev| !rev.starts_with('-')) {
            let spec = format!("{rev}^{{commit}}");
            let args = [
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &spec,
            ];
            let hash = run(&args).await.ok().map(text);
            (hash.filter(|hash| !hash.is_empty()), None)
        } else {
            let list = run(&["worktree", "list", "--porcelain", "-z"]).await.ok();
            let main = list.as_deref().and_then(crate::worktree_stats::main_branch);
            let mut merge = None;
            if let Some(main) = &main {
                let spec = format!("refs/heads/{main}");
                merge = run(&["merge-base", "HEAD", &spec]).await.ok().map(text);
            }
            (merge.filter(|hash| !hash.is_empty()), main)
        };
        let from = base.as_deref().unwrap_or("HEAD");
        let stat = run(&["diff", "--shortstat", from, "--"]).await.ok()?;
        let (added, deleted) =
            crate::worktree_stats::parse_shortstat(&String::from_utf8_lossy(&stat));
        // A detached HEAD has no branch; the row keeps the folder name.
        let branch = run(&["symbolic-ref", "--short", "-q", "HEAD"])
            .await
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .filter(|branch| !branch.is_empty());
        Some(crate::worktree_stats::WorktreeGit {
            branch,
            added,
            deleted,
            base_name: base_name.filter(|_| base.is_some()),
            base,
        })
    }

    /// `git rev-parse --git-common-dir` succeeds in `path`.
    async fn is_git_checkout(&self, path: PathBuf, cancel: &CancellationToken) -> bool {
        self.fs()
            .run(cancel, move || {
                anyhow::Ok(terminator_core::worktrees::common_dir(&path).is_ok())
            })
            .await
            .unwrap_or(false)
    }

    async fn stage_gutter_block(
        &self,
        job: Job,
        cancel: &CancellationToken,
        updates: &mut Vec<Update>,
    ) {
        let Job::StageGutterBlock {
            cwd,
            path,
            lines,
            text,
        } = job
        else {
            return;
        };
        let result = crate::diff::actions::stage_block(crate::diff::actions::StageBlockInput {
            service: self,
            cwd,
            path: path.clone(),
            lines,
            text,
            cancel,
        })
        .await
        .map_err(|error| format!("{error:#}"));
        updates.push(Update::GutterStaged(path, result));
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
                anyhow::Ok(crate::git_log::blame_with_contents(
                    &cwd,
                    &file,
                    Some(&text),
                ))
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
