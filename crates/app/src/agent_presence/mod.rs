//! Shared agent identity/status presentation. One model backs workspace tabs,
//! terminal-strip tabs, split-pane headers, session rows, and agent rows so
//! identity, lifecycle, unread state, and diagnostics render consistently.
mod cache;
pub(crate) use cache::AttentionInput;
pub(crate) use cache::GetInput;
pub(crate) use cache::ago;
mod counts;
mod owner;
#[cfg(test)]
mod presence_tests;
mod presentation;
mod queries;
mod resume;
pub(crate) use cache::PresentationCache;
pub(crate) use counts::{AttentionCounts, StateCounts, attention_status_icon, notice_pending};
pub(crate) use presentation::AgentPresentation;
#[cfg(test)]
pub(crate) use presentation::row_state;
pub(crate) use queries::notice_preview;
