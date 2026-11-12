//! Git history, merge and worktree UI.
pub(super) mod git_log;
pub(super) mod local_history;
pub(super) mod merge;
pub(super) mod revision_compare;
pub(super) mod task_changes;
mod task_form;
mod task_page;
#[cfg(test)]
mod task_tests;
pub(super) mod worktree_stats;
pub(super) mod worktree_ui;
