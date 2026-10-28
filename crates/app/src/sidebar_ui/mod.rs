//! Project, file, Git, notification, and session-info sidebar rendering.
#[cfg(all(test, feature = "test-support"))]
pub(crate) use git_panel::GitPanelInput;
mod agents;
mod attention;
mod explorer;
pub(crate) use explorer::SessionRowInTabInput;
mod explorer_projects;
mod explorer_prompts;
mod explorer_rows;
pub(crate) use explorer_rows::ExplorerToggleInput;
mod explorer_select;
pub(crate) use explorer_select::ExplorerKeysInput;
mod git_panel;
mod perforce_panel;
mod problems;
mod structure;
pub(crate) use structure::StructureState;
#[cfg(test)]
mod sidebar_tests;

pub(crate) use attention::{
    AttentionAction, AttentionCard, NoticeGroup, attention_card, group_notices, notice_pending,
    notice_rank, notice_waiting, state_color,
};
pub(crate) use explorer::ExplorerReveal;
pub(crate) use explorer_prompts::GitConfirm;
pub(crate) use explorer_rows::explorer_query_keeps_file;
#[cfg(all(test, feature = "test-support"))]
pub(crate) use git_panel::git_panel;
#[cfg(test)]
pub(crate) use git_panel::{GitClickActionInput, git_click_action};
pub(crate) use git_panel::{GitPanelOutcome, PreparedGit};
