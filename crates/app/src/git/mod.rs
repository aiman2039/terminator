//! Git history, merge and worktree UI.
pub(super) mod git_log;
#[cfg(test)]
mod include_tests;
pub(super) mod local_history;
pub(super) mod merge;
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
