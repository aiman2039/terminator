use terminator_core::AgentState;
#[cfg(test)]
use terminator_core::State;
/// Status glyph per lifecycle state. The brand icon beside it identifies the
/// agent; this glyph only reports hook lifecycle.
#[must_use]
pub(crate) fn attention_status_icon(state: AgentState) -> &'static str {
    match state {
        AgentState::WaitingInput => "MessageCircleQuestion",
        AgentState::WaitingPermission => "ShieldQuestion",
        AgentState::Completed => "CircleCheck",
        AgentState::Failed => "CircleAlert",
        AgentState::Running => "LoaderCircle",
        AgentState::Stopped => "CircleStop",
        AgentState::Unknown => "CircleQuestion",
    }
}

/// Pending-attention breakdown. Input requests, permission requests, and
/// failures stay distinct in workspace-tab aggregates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AttentionCounts {
    pub input: usize,
    pub permission: usize,
    pub failed: usize,
}

impl AttentionCounts {
    #[must_use]
    pub fn waiting(self) -> usize {
        self.input.saturating_add(self.permission)
    }
    #[must_use]
    pub fn total(self) -> usize {
        self.input
            .saturating_add(self.permission)
            .saturating_add(self.failed)
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.total() == 0
    }
}

#[must_use]
pub(crate) fn notice_pending(notice: &terminator_core::Notification, now: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= now
}

/// Spec for one tab's attention. The GUI uses [`PresentationCache::attention`].
#[cfg(test)]
#[must_use]
pub(crate) fn tab_attention(state: &State, session_ids: &[String], now: u64) -> AttentionCounts {
    let live: std::collections::HashSet<_> = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .map(|s| s.id.as_str())
        .collect();
    let mut counts = AttentionCounts::default();
    for notice in &state.notifications {
        if !live.contains(notice.session_id.as_str())
            || !session_ids.contains(&notice.session_id)
            || !notice_pending(notice, now)
        {
            continue;
        }
        match notice.state {
            AgentState::WaitingInput => counts.input = counts.input.saturating_add(1),
            AgentState::WaitingPermission => {
                counts.permission = counts.permission.saturating_add(1);
            }
            AgentState::Failed => counts.failed = counts.failed.saturating_add(1),
            _ => {}
        }
    }
    counts
}
