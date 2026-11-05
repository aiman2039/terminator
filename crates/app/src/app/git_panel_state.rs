use std::path::PathBuf;

use terminator_core::async_service::CancellationToken;

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
    pub(crate) history: bool,
    pub(crate) branches: Vec<String>,
    pub(crate) log: Vec<(String, String)>,
    pub(crate) shelves: Vec<(String, String)>,
    pub(crate) list_root: Option<PathBuf>,
    pub(crate) collapse: u64,
    /// Branch comparison for the current Git root (upstream, ahead/behind,
    /// and files committed since the base ref).
    pub(crate) compare: Option<workspace_ops::CompareData>,
    pub(crate) compare_root: Option<PathBuf>,
    /// User-picked base ref; falls back to the upstream branch when `None`.
    pub(crate) base_ref: Option<String>,
    /// Destructive Git action awaiting confirmation on the inline bar.
    pub(crate) confirm: Option<sidebar_ui::GitConfirm>,
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
    /// The draft and its Amend choice, given back when the commit itself
    /// does not land.
    message: String,
    amend: bool,
    phase: sidebar_ui::CommitPhase,
    cancelling: bool,
    cancel: CancellationToken,
}

impl CommitRun {
    pub(crate) fn progress(&self) -> sidebar_ui::CommitProgress<'_> {
        sidebar_ui::CommitProgress {
            phase: self.phase,
            cancelling: self.cancelling,
            root: &self.root,
        }
    }
}

impl GitPanelState {
    /// Starts a commit run in `root` unless one is in flight: the
    /// double-start guard. Returns the token that stops its job.
    pub(crate) fn start_commit(
        &mut self,
        root: PathBuf,
        message: String,
    ) -> Option<CancellationToken> {
        if self.run.is_some() {
            return None;
        }
        let cancel = CancellationToken::new();
        self.run = Some(CommitRun {
            root,
            message,
            amend: self.amend,
            phase: sidebar_ui::CommitPhase::Committing,
            cancelling: false,
            cancel: cancel.clone(),
        });
        self.commit_failure = None;
        Some(cancel)
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
        git.start_commit("/repo".into(), "msg".into()).unwrap()
    }

    #[test]
    fn a_second_commit_is_refused_while_one_runs() {
        let mut git = GitPanelState::default();
        let first = running(&mut git);
        assert!(git.start_commit("/repo".into(), "next".into()).is_none());
        assert!(!first.is_cancelled());
        assert!(git.finish_commit(&Ok("Committed".into())));
        assert!(git.start_commit("/repo".into(), "next".into()).is_some());
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
