use super::super::*;
use super::counts::AttentionCounts;
use super::owner::OwnerSupport;
use super::queries::{DiagnosticsTextInput, diagnostics_text};
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
    /// Short trailing text for a session row, such as `waiting 2m`.
    /// Computed per frame: the age must not be cached.
    pub fn row_state(&self, now: u64) -> Option<String> {
        let since = self
            .diagnostics
            .as_ref()
            .and_then(|d| d.hook.as_ref())
            .map(|hook| hook.updated);
        row_state(self.lifecycle?, since, now)
    }

    pub fn diagnostics(&self, now: u64) -> String {
        let Some(d) = &self.diagnostics else {
            return "Unknown session".into();
        };
        diagnostics_text(DiagnosticsTextInput {
            session: &d.session,
            presence: d.presence.as_ref(),
            detected: &d.detected,
            hook: d.hook.as_ref(),
            support: d.support,
            verified: self.verified,
            hook_linked: d.hook_linked,
            status_label: &self.status_label,
            unread: self.unread,
            now,
        })
    }
}

/// Trailing row text for a hook state. Waiting shows its age, because a
/// long wait needs attention. No text for unknown or stopped agents.
pub(crate) fn row_state(state: AgentState, since: Option<u64>, now: u64) -> Option<String> {
    let label = match state {
        AgentState::WaitingInput | AgentState::WaitingPermission => {
            return Some(match since {
                Some(since) => format!("waiting {}", short_age(since, now)),
                None => "waiting".into(),
            });
        }
        AgentState::Running => "working",
        AgentState::Completed => "done",
        AgentState::Failed => "failed",
        AgentState::Unknown | AgentState::Stopped => return None,
    };
    Some(label.into())
}

/// `12s`, `4m` or `2h`.
pub(crate) fn short_age(since: u64, now: u64) -> String {
    let age = now.saturating_sub(since);
    if age < 60 {
        format!("{age}s")
    } else if age < 3600 {
        format!("{}m", age / 60)
    } else {
        format!("{}h", age / 3600)
    }
}
