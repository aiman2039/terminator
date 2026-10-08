//! Project, file, Git, notification, and session-info sidebar rendering.
mod agents;
mod attention;
mod explorer;
mod explorer_projects;
mod explorer_prompts;
mod explorer_rows;
mod git_panel;
#[cfg(test)]
mod sidebar_tests;

pub(crate) use attention::{
    AttentionAction, AttentionCard, NoticeGroup, attention_card, group_notices, notice_pending,
    notice_rank, notice_waiting, state_color,
};
pub(crate) use explorer_rows::explorer_query_keeps_file;
#[cfg(test)]
pub(crate) use git_panel::{GitPanelInput, git_click_action, git_panel};
pub(crate) use git_panel::{GitPanelOutcome, PreparedGit, git_color};
