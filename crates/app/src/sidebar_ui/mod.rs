//! Project, file, Git, notification, and session-info sidebar rendering.
#[cfg(all(test, feature = "test-support"))]
pub(crate) use git_panel::GitPanelInput;
mod agents;
mod attention;
mod explorer;
pub(crate) use explorer::SessionRowInTabInput;
mod explorer_file_menu;
mod explorer_projects;
#[cfg(test)]
pub(crate) use explorer_projects::RenameDraft;
pub(crate) use explorer_projects::{ProjectNames, TreeDrag, apply_project_names};
mod explorer_prompts;
mod explorer_rows;
pub(crate) use explorer_rows::ExplorerToggleInput;
mod explorer_select;
pub(crate) use explorer_select::ExplorerKeysInput;
mod session_select;
pub(crate) use session_select::SessionSelection;
mod git_panel;
mod perforce_panel;
pub(crate) use git_panel::publish_sync_run;
pub(crate) use perforce_panel::publish as publish_perforce_run;
mod problems;
pub(crate) mod structure;
pub(crate) use structure::StructureState;
#[cfg(test)]
mod sidebar_tests;

pub(crate) use attention::{
    AttentionAction, AttentionCard, NoticeGroup, attention_card, group_notices, notice_pending,
    notice_rank, notice_waiting, state_color,
};
pub(crate) use explorer::ExplorerReveal;
pub(crate) use explorer_prompts::{GitConfirm, PendingGitConfirm};
pub(crate) use explorer_rows::explorer_query_keeps_file;
#[cfg(all(test, feature = "test-support"))]
pub(crate) use git_panel::git_panel;
pub(crate) use git_panel::{
    CommitLock, CommitPhase, CommitPick, CommitProgress, GitPanelOutcome, GitTab, PreparedGit,
    commit_actions,
};
#[cfg(test)]
pub(crate) use git_panel::{GitClickActionInput, git_click_action};
