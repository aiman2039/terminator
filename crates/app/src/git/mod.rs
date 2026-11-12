//! Git history, merge and worktree UI.
pub(crate) mod branch_ui;
pub(super) mod git_log;
pub(super) mod local_history;
pub(super) mod merge;
mod preview_tab;
pub(super) mod revision_compare;
mod task_form;
pub(super) mod worktree_stats;
pub(super) mod worktree_ui;

/// GUI-only Git state that is not part of the Git panel's commit state.
#[derive(Default)]
pub(crate) struct GitExtras {
    /// The Git panel's reused diff tab.
    pub(crate) preview: preview_tab::Preview,
    /// A branch action waiting for its name or confirmation.
    pub(crate) dialog: Option<branch_ui::PendingBranchDialog>,
}
