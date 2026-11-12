//! Git history, merge and worktree UI.
pub(crate) mod branch_ui;
mod draft_message;
pub(super) mod git_log;
#[cfg(test)]
mod include_tests;
pub(super) mod local_history;
pub(super) mod merge;
mod preview_tab;
mod pull_request;
pub(super) mod revision_compare;
pub(super) mod task_changes;
mod task_form;
#[cfg(test)]
#[cfg(feature = "test-support")]
mod task_form_tests;
pub(super) mod task_issue;
mod task_page;
#[cfg(test)]
mod task_tests;
pub(super) mod worktree_cleanup;
pub(super) mod worktree_include;
pub(super) mod worktree_stats;
pub(super) mod worktree_ui;

/// GUI-only Git state that is not part of the Git panel's commit state.
#[derive(Default)]
pub(crate) struct GitExtras {
    /// The Git panel's reused diff tab.
    pub(crate) preview: preview_tab::Preview,
    /// A branch action waiting for its name or confirmation.
    pub(crate) dialog: Option<branch_ui::PendingBranchDialog>,
    /// The Git panel's PR tab.
    pub(crate) pull_request: pull_request::PrPanel,
    /// The Worktrees page: probes, ticks and Apply to source.
    pub(crate) cleanup: worktree_cleanup::Cleanup,
    /// Data of `CenterPage::LastTurn`.
    pub(crate) last_turn: Option<local_history::turn::LastTurn>,
    /// New task's issue or pull request lookup in flight.
    pub(crate) task_issue: task_issue::IssueFetch,
}
