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
    /// Counts one pending notice; states other than input, permission,
    /// and failure are not attention.
    pub fn count(&mut self, state: AgentState) {
        match state {
            AgentState::WaitingInput => self.input = self.input.saturating_add(1),
            AgentState::WaitingPermission => {
                self.permission = self.permission.saturating_add(1);
            }
            AgentState::Failed => self.failed = self.failed.saturating_add(1),
            _ => {}
        }
    }
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

/// Live agents by hook state, for the state counters on project, tab group
/// and worktree rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StateCounts {
    pub waiting: usize,
    pub failed: usize,
    pub working: usize,
    pub done: usize,
}

impl StateCounts {
    pub fn add(&mut self, state: AgentState) {
        let slot = match state {
            AgentState::WaitingInput | AgentState::WaitingPermission => &mut self.waiting,
            AgentState::Failed => &mut self.failed,
            AgentState::Running => &mut self.working,
            AgentState::Completed => &mut self.done,
            AgentState::Unknown | AgentState::Stopped => return,
        };
        *slot = slot.saturating_add(1);
    }

    #[must_use]
    pub fn of(states: impl IntoIterator<Item = AgentState>) -> Self {
        let mut counts = Self::default();
        for state in states {
            counts.add(state);
        }
        counts
    }

    /// Counter order: waiting, failed, working, done.
    #[must_use]
    pub fn ordered(self) -> [(AgentState, usize); 4] {
        [
            (AgentState::WaitingPermission, self.waiting),
            (AgentState::Failed, self.failed),
            (AgentState::Running, self.working),
            (AgentState::Completed, self.done),
        ]
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
