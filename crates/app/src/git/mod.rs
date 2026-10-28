//! Git history, merge and worktree UI.
pub(super) mod git_log;
#[cfg(any(test, target_os = "macos"))]
pub(super) mod merge;
pub(super) mod worktree_ui;
