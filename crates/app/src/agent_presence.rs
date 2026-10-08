//! Shared agent identity/status presentation. One model backs workspace tabs,
//! terminal-strip tabs, split-pane headers, session rows, and agent rows so
//! identity, lifecycle, unread state, and diagnostics render consistently.
mod cache;
mod counts;
mod owner;
#[cfg(test)]
mod presence_tests;
mod presentation;
mod queries;
pub(crate) use cache::PresentationCache;
pub(crate) use counts::{AttentionCounts, attention_status_icon, notice_pending};
pub(crate) use presentation::AgentPresentation;
pub(crate) use queries::notice_preview;
