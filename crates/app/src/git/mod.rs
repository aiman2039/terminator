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
mod task_page;
#[cfg(test)]
mod task_tests;
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
}
