//! Agent presence for the GUI: cached presentations, tab attention counts,
//! and the global order that Next attention cycles through.
use crate::agent_presence::{AttentionInput, GetInput};
use std::collections::{HashMap, HashSet};
use terminator_core::*;

use super::super::*;

impl App {
    pub(crate) fn present_session(
        &self,
        id: &str,
    ) -> std::sync::Arc<agent_presence::AgentPresentation> {
        self.presentations.borrow_mut().get(GetInput {
            state: &self.state,
            id,
            now: now(),
            fresh: self.services.presence_fresh(),
        })
    }

    pub(crate) fn reconcile_presentations(&mut self) {
        let fresh = self.services.presence_fresh();
        let moment = now();
        self.presentations
            .get_mut()
            .reconcile(&self.state, moment, fresh);
    }

    pub(crate) fn cached_tab_attention(
        &self,
        session_ids: &[String],
    ) -> agent_presence::AttentionCounts {
        self.presentations.borrow_mut().attention(AttentionInput {
            state: &self.state,
            session_ids,
            now: now(),
            fresh: self.services.presence_fresh(),
        })
    }

    /// Global cycle order for attention navigation: distinct live terminals
    /// with pending permission/input requests first, then failures; oldest
    /// first within each group. Dismissal and snooze are respected.
    pub(crate) fn attention_order(&self) -> Vec<String> {
        let live: HashSet<&str> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .map(|s| s.id.as_str())
            .collect();
        let mut earliest_waiting: HashMap<&str, u64> = HashMap::new();
        let mut earliest_failed: HashMap<&str, u64> = HashMap::new();
        for notice in &self.state.notifications {
            if !live.contains(notice.session_id.as_str())
                || !agent_presence::notice_pending(notice, now())
            {
                continue;
            }
            let slot = match notice.state {
                AgentState::WaitingInput | AgentState::WaitingPermission => &mut earliest_waiting,
                AgentState::Failed => &mut earliest_failed,
                _ => continue,
            };
            slot.entry(notice.session_id.as_str())
                .and_modify(|seen| *seen = (*seen).min(notice.created))
                .or_insert(notice.created);
        }
        let mut order: Vec<(&str, u64)> = earliest_waiting.into_iter().collect();
        order.sort_by_key(|(id, created)| (*created, *id));
        let mut failed: Vec<(&str, u64)> = earliest_failed
            .into_iter()
            .filter(|(session, _)| !order.iter().any(|(waiting, _)| waiting == session))
            .collect();
        failed.sort_by_key(|(id, created)| (*created, *id));
        order
            .into_iter()
            .chain(failed)
            .map(|(session, _)| session.to_string())
            .collect()
    }

    /// Reveal the next terminal needing attention, cycling globally through
    /// [`Self::attention_order`] and reusing session navigation.
    pub(crate) fn next_attention(&mut self) {
        let order = self.attention_order();
        if order.is_empty() {
            self.info = Some("No agents need attention".into());
            return;
        }
        let next = self
            .active_session
            .as_ref()
            .and_then(|active| order.iter().position(|id| id == active))
            .map_or(0, |index| {
                index
                    .saturating_add(1)
                    .checked_rem(order.len())
                    .unwrap_or(0)
            });
        let Some(target) = order.get(next).cloned() else {
            return;
        };
        self.go_session(&target);
    }
}
