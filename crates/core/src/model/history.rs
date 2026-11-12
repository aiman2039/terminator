//! Compaction of a historical (not live) state. Moved out of `mod.rs`
//! unchanged, plus the live-only program status.
use super::*;

impl State {
    /// Drop transient records that only matter to a live owner so a historical
    /// state stays small when it is loaded for aggregation. Pending attention
    /// is never dropped; only already dismissed/resolved notifications beyond
    /// the retention cap are removed. Sessions and resumable agents are kept.
    pub fn compact_history(&mut self) {
        self.recent_events.clear();
        self.presence.clear();
        self.terminal_titles.clear();
        self.program_status.clear();
        let len = self.notifications.len();
        if len > MAX_NOTIFICATIONS {
            let cutoff = len.saturating_sub(MAX_NOTIFICATIONS);
            let mut index = 0;
            self.notifications.retain(|n| {
                let keep = index >= cutoff || !(n.dismissed || n.resolved);
                index = index.saturating_add(1);
                keep
            });
        }
        let len = self.agents.len();
        if len > MAX_AGENTS {
            let cutoff = len.saturating_sub(MAX_AGENTS);
            let mut index = 0;
            self.agents.retain(|a| {
                // Keep live/resumable agents and the most recent records; a
                // resumable handle must never be trimmed.
                let keep = index >= cutoff || a.resumable() || a.state == AgentState::Running;
                index = index.saturating_add(1);
                keep
            });
        }
    }
}
