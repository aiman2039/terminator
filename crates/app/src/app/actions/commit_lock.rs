//! The one guard for the actions that a running commit locks. Every sender
//! of an index, `HEAD`, or stash change (Git panel, Find Action, diff tabs,
//! gutter, merge) asks it first: a refused action sends no job and says why
//! in the status bar, and the controls show why they are disabled.
use crate::app::git_panel_state::RepoTarget;
use crate::services::ContextData;
use crate::workspace_ops::{COMMIT_RUNNING, COMMIT_WAIT, CommitRisk};
use crate::*;

impl App {
    /// Whether a running commit locks an action of `risk` in the
    /// repository whose canonical worktree root is `root`. `None` is a
    /// folder whose repository is unknown (see `CommitRun::locks`).
    pub(crate) fn commit_blocks(&self, root: Option<&Path>, risk: CommitRisk) -> bool {
        let target = root.map(|root| RepoTarget::new(root, self.context.as_ref()));
        self.git.commit_locks(target, risk)
    }

    /// Refuses an action that a running commit locks: the status bar says
    /// why, and the caller sends nothing. True when refused. The bar shows
    /// an error before a notice, so an older error (often the last commit's
    /// failure) gives way: a key or Find Action has no other feedback.
    pub(crate) fn refuse_during_commit(&mut self, root: Option<&Path>, risk: CommitRisk) -> bool {
        let refused = self.commit_blocks(root, risk);
        if refused {
            self.error = None;
            self.info = Some(COMMIT_RUNNING.into());
        }
        refused
    }

    /// Hover text of a control that a running commit disables, if it does.
    pub(crate) fn commit_wait(
        &self,
        root: Option<&Path>,
        risk: CommitRisk,
    ) -> Option<&'static str> {
        self.commit_blocks(root, risk).then_some(COMMIT_WAIT)
    }

    /// What the running commit locks in the Git panel of `context`. The
    /// panel paints with the context taken out of the app.
    pub(crate) fn git_panel_lock(&self, context: &ContextData) -> sidebar_ui::CommitLock {
        let wait = |risk| {
            let target = context
                .root
                .as_deref()
                .map(|root| RepoTarget::new(root, Some(context)));
            self.git.commit_locks(target, risk).then_some(COMMIT_WAIT)
        };
        sidebar_ui::CommitLock {
            index: wait(CommitRisk::Index),
            stash: wait(CommitRisk::Stash),
            head: wait(CommitRisk::Head),
        }
    }
}
