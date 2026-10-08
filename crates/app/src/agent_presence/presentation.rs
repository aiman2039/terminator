use super::super::*;
use super::counts::AttentionCounts;
use super::owner::OwnerSupport;
use super::queries::diagnostics_text;
use terminator_core::agents::{self};
/// Complete presentation for one terminal session.
#[derive(Clone, Debug)]
pub(crate) struct AgentPresentation {
    /// Stable brand glyph, e.g. `AgentCodex` or `AgentMultiple`.
    pub brand_icon: Option<&'static str>,
    /// Brand text, e.g. `Codex` or `3 agents`.
    pub brand_label: Option<String>,
    /// Verified live agent kinds for this terminal.
    pub detected_kinds: Vec<String>,
    /// A fresh verified observation exists (even when it reports no agents).
    pub verified: bool,
    /// True when at least one agent process is verified live.
    pub live: bool,
    /// Hook lifecycle, when any hook event was recorded.
    pub lifecycle: Option<AgentState>,
    /// Lifecycle label, or "Status unavailable" when unknown.
    pub status_label: String,
    pub status_icon: &'static str,
    pub spin: bool,
    pub unread: usize,
    pub attention: AttentionCounts,
    /// Latest non-dismissed notice sentence. Absent when there is no hook
    /// lifecycle, or when the text only repeats the state label.
    pub notice_preview: Option<String>,
    /// Deferred tooltip inputs. Never contain command arguments.
    pub(super) diagnostics: Option<Diagnostics>,
}

#[derive(Clone, Debug)]
pub(super) struct Diagnostics {
    pub(super) session: Session,
    pub(super) presence: Option<agents::TerminalPresence>,
    pub(super) detected: Vec<agents::DetectedAgent>,
    pub(super) hook: Option<terminator_core::Agent>,
    pub(super) support: OwnerSupport,
    pub(super) hook_linked: bool,
}

impl AgentPresentation {
    pub fn diagnostics(&self, now: u64) -> String {
        let Some(d) = &self.diagnostics else {
            return "Unknown session".into();
        };
        diagnostics_text(
            &d.session,
            d.presence.as_ref(),
            &d.detected,
            d.hook.as_ref(),
            d.support,
            self.verified,
            d.hook_linked,
            &self.status_label,
            self.unread,
            now,
        )
    }
}
