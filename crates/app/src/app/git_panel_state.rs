use std::path::{Path, PathBuf};

use terminator_core::async_service::CancellationToken;

use crate::services::ContextData;
use crate::workspace_ops::CommitRisk;
use crate::{sidebar_ui, workspace_ops};

/// Sidebar Git panel state: the commit draft and its options, branch/log/
/// shelf lists for the current Git root, the branch comparison, and the
/// destructive action awaiting confirmation.
#[derive(Default)]
pub(crate) struct GitPanelState {
    pub(crate) commit: String,
    /// The commit (and push) job in flight. One runs at a time; the commit
    /// buttons wait for its reply.
    pub(crate) run: Option<CommitRun>,
    /// Summary line and output tail of the last failed commit or push.
    pub(crate) commit_failure: Option<String>,
    pub(crate) amend: bool,
    pub(crate) signoff: bool,
    /// The open header tab.
    pub(crate) tab: sidebar_ui::GitTab,
    pub(crate) branches: Vec<String>,
    pub(crate) log: Vec<(String, String)>,
    pub(crate) shelves: Vec<(String, String)>,
    pub(crate) list_root: Option<PathBuf>,
    /// Expand/collapse-all generation; a new one resets the tree's open state.
    pub(crate) collapse: u64,
    /// The last Collapse all is still in force (else everything starts open).
    pub(crate) tree_collapsed: bool,
    /// Branch comparison for the current Git root (upstream, ahead/behind,
    /// and files committed since the base ref).
    pub(crate) compare: Option<workspace_ops::CompareData>,
    pub(crate) compare_root: Option<PathBuf>,
    /// User-picked base ref; falls back to the upstream branch when `None`.
    pub(crate) base_ref: Option<String>,
    /// Destructive Git action awaiting confirmation on the inline bar.
    pub(crate) confirm: Option<sidebar_ui::PendingGitConfirm>,
    /// The Compare with… popup, while it is open.
    pub(crate) picker: Option<crate::revision_compare::ComparePicker>,
    /// Commit Log filter bars, by tab key.
    pub(crate) logs: std::collections::HashMap<String, crate::git_log::LogView>,
    /// Focus the commit message the next time the panel paints.
    pub(crate) focus_message: bool,
}

/// A commit (and push) job in flight.
pub(crate) struct CommitRun {
    root: PathBuf,
    /// The Git common dir of `root`'s repository, when known: Shelve,
    /// Unshelve, and Drop in any of its worktrees change the stash.
    common_dir: Option<PathBuf>,
    /// The draft and its Amend choice, given back when the commit itself
    /// does not land.
    message: String,
    amend: bool,
    phase: sidebar_ui::CommitPhase,
    cancelling: bool,
    cancel: CancellationToken,
}

/// A repository that an action writes: its canonical worktree root and,
/// when known, the Git common dir that its worktrees share.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RepoTarget<'a> {
    pub(crate) root: &'a Path,
    pub(crate) common_dir: Option<&'a Path>,
}

impl<'a> RepoTarget<'a> {
    /// `root`, with its common dir when `context` is that repository's.
    pub(crate) fn new(root: &'a Path, context: Option<&'a ContextData>) -> Self {
        let common_dir = context
            .filter(|context| context.root.as_deref() == Some(root))
            .and_then(|context| match context.git_dirs.as_slice() {
                [_, common] => Some(common.as_path()),
                _ => None,
            });
        Self { root, common_dir }
    }
}

impl CommitRun {
    pub(crate) fn progress(&self) -> sidebar_ui::CommitProgress<'_> {
        sidebar_ui::CommitProgress {
            phase: self.phase,
            cancelling: self.cancelling,
            root: &self.root,
        }
    }

    /// Whether an action of `risk` in `target` could change this commit
    /// or its push. Index and stash writers wait for `git commit` only:
    /// once the push starts, the commit landed. A `HEAD` writer waits for
    /// the push too, which pushes the branch `HEAD` names when it starts.
    ///
    /// Roots are canonical worktree toplevels (Git panel, diff source,
    /// gutter), so they compare exactly: another worktree or a nested
    /// repository has its own index and `HEAD`. Every worktree of a
    /// repository shares the stash, so stash writers also match by the
    /// common dir. What is unknown may be this repository, so it locks:
    /// a folder with no known root, or a common dir that one side lacks.
    pub(crate) fn locks(&self, target: Option<RepoTarget<'_>>, risk: CommitRisk) -> bool {
        let committing = self.phase == sidebar_ui::CommitPhase::Committing;
        match (risk, target) {
            (CommitRisk::None, _) => false,
            (CommitRisk::Index | CommitRisk::Stash, _) if !committing => false,
            (_, None) => true,
            (CommitRisk::Index | CommitRisk::Head, Some(target)) => target.root == self.root,
            (CommitRisk::Stash, Some(target)) => {
                target.root == self.root
                    || match (target.common_dir, self.common_dir.as_deref()) {
                        (Some(theirs), Some(ours)) => theirs == ours,
                        _ => true,
                    }
            }
        }
    }
}

impl GitPanelState {
    /// Starts a commit run in `root` unless one is in flight: the
    /// double-start guard. Returns the token that stops its job.
    pub(crate) fn start_commit(
        &mut self,
        root: PathBuf,
        common_dir: Option<PathBuf>,
        message: String,
    ) -> Option<CancellationToken> {
        if self.run.is_some() {
            return None;
        }
        let cancel = CancellationToken::new();
        self.run = Some(CommitRun {
            root,
            common_dir,
            message,
            amend: self.amend,
            phase: sidebar_ui::CommitPhase::Committing,
            cancelling: false,
            cancel: cancel.clone(),
        });
        self.commit_failure = None;
        Some(cancel)
    }

    /// Whether the running commit locks an action of `risk` in `target`
    /// (see [`CommitRun::locks`]).
    pub(crate) fn commit_locks(&self, target: Option<RepoTarget<'_>>, risk: CommitRisk) -> bool {
        self.run.as_ref().is_some_and(|run| run.locks(target, risk))
    }

    /// The commit landed and its push started. False when nothing runs.
    pub(crate) fn commit_pushing(&mut self) -> bool {
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        run.phase = sidebar_ui::CommitPhase::Pushing;
        true
    }

    /// Asks the job to stop: it sends Git's process group Ctrl-C (then
    /// kills what still runs), then replies. False when nothing runs or a
    /// cancel is already pending.
    pub(crate) fn cancel_commit(&mut self) -> bool {
        let Some(run) = self.run.as_mut().filter(|run| !run.cancelling) else {
            return false;
        };
        run.cancelling = true;
        run.cancel.cancel();
        true
    }

    /// The job's reply ends the run. A commit that did not land gives its
    /// message and Amend choice back to an empty draft; a failure keeps
    /// its output for the panel. Returns whether the commit landed.
    pub(crate) fn finish_commit(&mut self, result: &Result<String, String>) -> bool {
        let run = self.run.take();
        // A push starts only after its commit landed.
        let pushing = run
            .as_ref()
            .is_some_and(|run| run.phase == sidebar_ui::CommitPhase::Pushing);
        let landed = match result {
            Ok(_) => true,
            Err(error) => pushing || workspace_ops::commit_landed(error),
        };
        if let Err(error) = result
            && !workspace_ops::commit_cancelled(error)
        {
            self.commit_failure = Some(if landed && !workspace_ops::commit_landed(error) {
                workspace_ops::push_failed(error)
            } else {
                error.clone()
            });
        }
        if !landed
            && let Some(run) = run
            && self.commit.trim().is_empty()
        {
            self.commit = run.message;
            self.amend = run.amend;
        }
        landed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar_ui::CommitPhase;

    fn running(git: &mut GitPanelState) -> CancellationToken {
        let common = Some("/repo/.git".into());
        git.start_commit("/repo".into(), common, "msg".into())
            .unwrap()
    }

    #[test]
    fn a_second_commit_is_refused_while_one_runs() {
        let mut git = GitPanelState::default();
        let first = running(&mut git);
        assert!(
            git.start_commit("/repo".into(), None, "next".into())
                .is_none()
        );
        assert!(!first.is_cancelled());
        assert!(git.finish_commit(&Ok("Committed".into())));
        assert!(
            git.start_commit("/repo".into(), None, "next".into())
                .is_some()
        );
    }

    fn target<'a>(root: &'a str, common_dir: Option<&'a str>) -> Option<RepoTarget<'a>> {
        Some(RepoTarget {
            root: Path::new(root),
            common_dir: common_dir.map(Path::new),
        })
    }

    #[test]
    fn a_commit_locks_index_writers_of_its_worktree_and_stash_writers_of_its_repository() {
        let mut git = GitPanelState::default();
        assert!(!git.commit_locks(target("/repo", None), CommitRisk::Index));
        running(&mut git);
        let (index, stash) = (CommitRisk::Index, CommitRisk::Stash);
        assert!(git.commit_locks(target("/repo", None), index));
        assert!(git.commit_locks(target("/repo", None), stash));
        assert!(!git.commit_locks(target("/repo", None), CommitRisk::None));
        // A linked worktree has its own index and HEAD but shares the stash.
        let linked = target("/linked", Some("/repo/.git"));
        assert!(!git.commit_locks(linked, index));
        assert!(git.commit_locks(linked, stash));
        // A nested repository and another repository are not locked.
        for other in [
            target("/repo/vendor/lib", Some("/repo/vendor/lib/.git")),
            target("/other", Some("/other/.git")),
        ] {
            assert!(!git.commit_locks(other, index));
            assert!(!git.commit_locks(other, stash));
        }
        // What is unknown may be this repository: a folder with no known
        // root, or a stash whose common dir is unknown.
        assert!(git.commit_locks(None, index));
        assert!(!git.commit_locks(None, CommitRisk::None));
        assert!(!git.commit_locks(target("/other", None), index));
        assert!(git.commit_locks(target("/other", None), stash));
        // The push and the end of the run lock nothing.
        git.commit_pushing();
        assert!(!git.commit_locks(target("/repo", None), index));
        assert!(!git.commit_locks(None, stash));
        git.finish_commit(&Ok("Committed and pushed".into()));
        assert!(!git.commit_locks(None, index));
    }

    #[test]
    fn a_repo_target_takes_the_common_dir_of_its_own_context_only() {
        let context = ContextData {
            cwd: "/linked/src".into(),
            root: Some("/linked".into()),
            perforce: None,
            git_dirs: vec!["/repo/.git/worktrees/linked".into(), "/repo/.git".into()],
            branch: "main".into(),
            changes: vec![],
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::default(),
            error: None,
        };
        let own = RepoTarget::new(Path::new("/linked"), Some(&context));
        assert_eq!(own.common_dir, Some(Path::new("/repo/.git")));
        let other = RepoTarget::new(Path::new("/other"), Some(&context));
        assert_eq!(other.common_dir, None);
        let partial = ContextData {
            git_dirs: vec!["/repo/.git".into()],
            ..context
        };
        let unknown = RepoTarget::new(Path::new("/linked"), Some(&partial));
        assert_eq!(unknown.common_dir, None, "one dir is ambiguous");
    }

    #[test]
    fn a_branch_switch_waits_for_the_push_as_well() {
        let mut git = GitPanelState::default();
        running(&mut git);
        let head = CommitRisk::Head;
        assert!(git.commit_locks(target("/repo", None), head));
        assert!(git.commit_pushing());
        // `git push` with no refspec reads HEAD when it starts.
        assert!(git.commit_locks(target("/repo", None), head));
        assert!(git.commit_locks(None, head), "an unknown root may be it");
        let linked = target("/linked", Some("/repo/.git"));
        assert!(
            !git.commit_locks(linked, head),
            "another worktree, another HEAD"
        );
        assert!(!git.commit_locks(target("/repo", None), CommitRisk::Index));
        git.finish_commit(&Ok("Committed and pushed".into()));
        assert!(!git.commit_locks(target("/repo", None), head));
    }

    #[test]
    fn a_pending_cancel_keeps_the_lock_until_the_job_replies() {
        let mut git = GitPanelState::default();
        running(&mut git);
        // Git and its hooks are still stopping (a hook manager may be
        // putting stashed edits back), and the commit can still land.
        assert!(git.cancel_commit());
        for risk in [CommitRisk::Index, CommitRisk::Stash, CommitRisk::Head] {
            assert!(git.commit_locks(target("/repo", None), risk), "{risk:?}");
        }
        git.finish_commit(&Err(workspace_ops::COMMIT_CANCELLED.into()));
        assert!(!git.commit_locks(target("/repo", None), CommitRisk::Index));
    }

    #[test]
    fn cancel_stops_the_job_once_and_its_reply_ends_the_run() {
        let mut git = GitPanelState::default();
        assert!(!git.cancel_commit(), "nothing runs");
        let token = running(&mut git);
        assert_eq!(
            git.run.as_ref().map(|run| run.progress().phase),
            Some(CommitPhase::Committing)
        );
        assert!(git.cancel_commit());
        assert!(token.is_cancelled());
        assert!(
            git.run
                .as_ref()
                .is_some_and(|run| run.progress().cancelling)
        );
        assert!(!git.cancel_commit(), "a cancel is already pending");
        // The run stays until the job confirms Git is gone.
        assert!(git.run.is_some());
        let landed = git.finish_commit(&Err(workspace_ops::COMMIT_CANCELLED.into()));
        assert!(!landed);
        assert!(git.run.is_none());
        assert_eq!(git.commit, "msg", "the draft comes back");
        assert!(git.commit_failure.is_none(), "a cancel is not a failure");
        assert!(!git.cancel_commit());
    }

    #[test]
    fn a_cancelled_push_keeps_the_landed_commit_out_of_the_draft() {
        let mut git = GitPanelState::default();
        running(&mut git);
        assert!(git.commit_pushing());
        assert_eq!(
            git.run.as_ref().map(|run| run.progress().phase),
            Some(CommitPhase::Pushing)
        );
        assert!(git.cancel_commit());
        assert!(git.finish_commit(&Err(workspace_ops::PUSH_CANCELLED.into())));
        assert!(git.commit.is_empty());
        assert!(git.commit_failure.is_none());
        assert!(!git.commit_pushing(), "no run left to update");
    }

    #[test]
    fn a_failed_commit_keeps_its_output_and_gives_the_draft_back() {
        let mut git = GitPanelState::default();
        running(&mut git);
        let error = "Commit failed (exit status: 1)\nhook said no";
        assert!(!git.finish_commit(&Err(error.into())));
        assert_eq!(git.commit, "msg");
        assert_eq!(git.commit_failure.as_deref(), Some(error));
        // A draft typed meanwhile is never replaced.
        running(&mut git);
        assert!(
            git.commit_failure.is_none(),
            "a new run clears the old failure"
        );
        git.commit = "typed".into();
        git.finish_commit(&Err(error.into()));
        assert_eq!(git.commit, "typed");
    }

    #[test]
    fn an_amend_that_did_not_land_gives_amend_back_with_its_message() {
        let mut git = GitPanelState {
            amend: true,
            ..GitPanelState::default()
        };
        running(&mut git);
        // The panel clears Amend once the run starts.
        git.amend = false;
        assert!(!git.finish_commit(&Err(workspace_ops::COMMIT_CANCELLED.into())));
        assert_eq!(git.commit, "msg");
        assert!(git.amend, "committing again amends again");
    }

    #[test]
    fn a_push_that_ends_without_a_reply_still_counts_the_commit() {
        let mut git = GitPanelState::default();
        running(&mut git);
        git.commit_pushing();
        assert!(git.finish_commit(&Err("Service task panicked".into())));
        assert!(git.commit.is_empty());
        assert!(
            git.commit_failure
                .as_deref()
                .is_some_and(|text| text.starts_with(workspace_ops::PUSH_FAILED))
        );
    }
}
