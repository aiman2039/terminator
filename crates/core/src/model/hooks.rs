//! Agent hook events: validation, deduplication, agent records, and notifications.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalNotice {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub body: String,
    pub created: u64,
    pub dismissed: bool,
}

impl State {
    /// An agent quota alert in the Agents inbox. Its own opt-in in the GUI,
    /// so the terminal notice setting does not apply.
    pub fn quota_notice(
        &mut self,
        session: &str,
        title: &str,
    ) -> Result<Option<String>, StateError> {
        self.require_live_session(session)?;
        self.push_terminal_notice(session, title, "")
    }

    pub fn apply_hook(&mut self, e: HookEvent) -> Result<Option<String>, StateError> {
        self.validate_hook_event(&e)?;
        let existing = self.hook_existing(&e);
        if self.hook_should_skip(&e, existing) {
            return Ok(None);
        }
        self.record_hook_event(&e);
        let repeated_state = existing
            .is_some_and(|i| self.agents.get(i).is_some_and(|a| a.state == e.state))
            && e.request_id.is_none();
        self.upsert_hook_agent(&e, existing);
        if existing.is_none() {
            self.cap_agents();
        }
        self.record_agent_usage(&e);
        self.resolve_hook_notifications(&e);
        self.revision = self.revision.saturating_add(1);
        if repeated_state || !e.state.actionable() || !self.settings.events.contains(&e.state) {
            return Ok(None);
        }
        if let Some(request) = &e.request_id
            && self.notifications.iter().any(|n| {
                n.session_id == e.terminal_session_id
                    && n.invocation_id == e.agent_invocation_id
                    && n.request_id.as_ref() == Some(request)
                    && n.state == e.state
            })
        {
            return Ok(None);
        }
        Ok(self.push_hook_notification(e))
    }

    /// A usage-only event (`Request::HookUsage`): merges `usage` into an
    /// agent this state already knows. The event's state, summary, request,
    /// sequence and process are ignored: it never creates an agent and never
    /// changes lifecycle, notices, presence or notifications. Returns whether
    /// the usage changed.
    ///
    /// # Errors
    /// Fails for an invalid event or an unknown or stopped agent.
    pub fn apply_hook_usage(&mut self, e: &HookEvent) -> Result<bool, StateError> {
        self.validate_hook_event(e)?;
        let known = self
            .hook_existing(e)
            .and_then(|i| self.agents.get(i))
            .is_some_and(|a| a.state != AgentState::Stopped);
        if !known {
            return Err(StateError::UnknownAgent {
                session: e.terminal_session_id.clone(),
            });
        }
        let Some(usage) = e.usage.as_ref().filter(|u| !u.is_empty()) else {
            return Ok(false);
        };
        let entry = self
            .agent_usage
            .entry(e.agent_invocation_id.clone())
            .or_default();
        let before = entry.clone();
        entry.merge(usage);
        let changed = *entry != before;
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        Ok(changed)
    }

    fn validate_hook_event(&self, e: &HookEvent) -> Result<(), StateError> {
        if e.protocol_version != PROTOCOL_VERSION {
            return Err(StateError::UnsupportedHookVersion {
                version: e.protocol_version,
            });
        }
        if !self
            .sessions
            .iter()
            .any(|s| s.id == e.terminal_session_id && s.lifecycle.live())
        {
            return Err(StateError::UnknownOrEndedSession {
                session: e.terminal_session_id.clone(),
            });
        }
        if e.event_id.is_empty() || e.agent_invocation_id.is_empty() || e.agent_kind.is_empty() {
            return Err(StateError::MissingEventIdentity {
                session: e.terminal_session_id.clone(),
            });
        }
        if e.summary.len() > 4096
            || e.details.len() > 65536
            || e.event_id.len() > 256
            || e.agent_invocation_id.len() > 512
            || e.agent_kind.len() > 128
            || e.request_id.as_ref().is_some_and(|r| r.len() > 512)
            || e.provider_session_id
                .as_ref()
                .is_some_and(|p| p.len() > 512)
            || e.resume.as_ref().is_some_and(resume_too_large)
        {
            return Err(StateError::HookTooLarge {
                event: e.event_id.chars().take(256).collect(),
            });
        }
        Ok(())
    }

    fn hook_existing(&self, e: &HookEvent) -> Option<usize> {
        self.agents.iter().position(|a| {
            a.invocation_id == e.agent_invocation_id && a.session_id == e.terminal_session_id
        })
    }

    fn hook_should_skip(&self, e: &HookEvent, existing: Option<usize>) -> bool {
        if self.recent_events.contains(&e.event_id) {
            return true;
        }
        if let Some(i) = existing {
            let Some(a) = self.agents.get(i) else {
                return true;
            };
            if a.state == AgentState::Stopped {
                return true;
            }
            if let (Some(old), Some(new)) = (a.sequence, e.sequence)
                && new <= old
            {
                return true;
            }
        }
        false
    }

    fn record_hook_event(&mut self, e: &HookEvent) {
        self.recent_events.push(e.event_id.clone());
        if self.recent_events.len() > MAX_RECENT_EVENTS {
            self.recent_events.drain(
                ..self
                    .recent_events
                    .len()
                    .saturating_sub(MAX_RECENT_EVENTS / 2),
            );
        }
    }

    fn upsert_hook_agent(&mut self, e: &HookEvent, existing: Option<usize>) {
        let mut agent = existing
            .and_then(|i| self.agents.get(i).cloned())
            .unwrap_or(Agent {
                invocation_id: e.agent_invocation_id.clone(),
                session_id: e.terminal_session_id.clone(),
                kind: e.agent_kind.clone(),
                provider_session_id: None,
                state: AgentState::Unknown,
                sequence: None,
                updated: now(),
                resume: None,
                // A new invocation never inherits an earlier invocation's process link.
                process: None,
            });
        agent.state = e.state;
        // An unsequenced event keeps the last sequence so out-of-order
        // checks still reject older retries.
        if e.sequence.is_some() {
            agent.sequence = e.sequence;
        }
        agent.updated = now();
        // Link lifecycle to a detected process only after verifying the
        // helper-supplied identity against a fresh observation of the same
        // kind in the same terminal. Unlinked events keep existing delivery.
        if let Some(identity) = &e.process {
            let observed = now();
            if self.presence.iter().any(|p| {
                p.session_id == e.terminal_session_id
                    && p.outcome == agents::PresenceOutcome::Verified
                    && agents::observation_fresh(p.observed_at, observed)
                    && p.agents
                        .iter()
                        .any(|a| a.kind == e.agent_kind && a.process == *identity)
            }) {
                agent.process = Some(identity.clone());
            }
        }
        if e.provider_session_id.is_some() {
            agent.provider_session_id.clone_from(&e.provider_session_id);
        }
        if e.resume.is_some() {
            agent.resume.clone_from(&e.resume);
        }
        if let Some(i) = existing
            && let Some(slot) = self.agents.get_mut(i)
        {
            *slot = agent;
        } else {
            self.agents.push(agent);
        }
    }

    /// Bounds `agents` in a live daemon. Above `MAX_AGENTS`, drops the
    /// oldest records that no longer matter: not resumable, in a session
    /// that is no longer live (its hook events are refused, so a dropped
    /// record cannot come back), and with no pending notification.
    fn cap_agents(&mut self) {
        let excess = self.agents.len().saturating_sub(MAX_AGENTS);
        if excess == 0 {
            return;
        }
        let sessions = &self.sessions;
        let notifications = &self.notifications;
        let mut candidates: Vec<(u64, String, String)> = self
            .agents
            .iter()
            .filter(|a| {
                !a.resumable()
                    && !sessions
                        .iter()
                        .any(|s| s.id == a.session_id && s.lifecycle.live())
                    && !notifications.iter().any(|n| {
                        n.invocation_id == a.invocation_id
                            && n.session_id == a.session_id
                            && !(n.dismissed || n.resolved)
                    })
            })
            .map(|a| (a.updated, a.session_id.clone(), a.invocation_id.clone()))
            .collect();
        candidates.sort_unstable();
        candidates.truncate(excess);
        self.agents.retain(|a| {
            !candidates
                .iter()
                .any(|(_, s, i)| *s == a.session_id && *i == a.invocation_id)
        });
    }

    /// Keeps the newest model and mode each agent reported. Display only:
    /// never a lifecycle or notification source.
    fn record_agent_usage(&mut self, e: &HookEvent) {
        if let Some(usage) = e.usage.as_ref().filter(|u| !u.is_empty()) {
            self.agent_usage
                .entry(e.agent_invocation_id.clone())
                .or_default()
                .merge(usage);
        }
        if !self.agent_usage.is_empty() {
            let agents = &self.agents;
            self.agent_usage
                .retain(|id, _| agents.iter().any(|a| &a.invocation_id == id));
        }
    }

    fn resolve_hook_notifications(&mut self, e: &HookEvent) {
        if matches!(
            e.state,
            AgentState::Running | AgentState::Completed | AgentState::Failed | AgentState::Stopped
        ) {
            for n in &mut self.notifications {
                if n.invocation_id == e.agent_invocation_id
                    && n.session_id == e.terminal_session_id
                    && !n.resolved
                {
                    n.resolved = true;
                    if self.settings.dismissal == Dismissal::OnResolve {
                        n.dismissed = true;
                    }
                }
            }
        }
    }

    fn push_hook_notification(&mut self, e: HookEvent) -> Option<String> {
        let nid = id();
        self.notifications.push(Notification {
            id: nid.clone(),
            session_id: e.terminal_session_id,
            invocation_id: e.agent_invocation_id,
            request_id: e.request_id,
            state: e.state,
            summary: e.summary,
            details: e.details,
            created: now(),
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        });
        // Bound completed history; never silently discard pending attention.
        if self.notifications.len() > MAX_NOTIFICATIONS
            && let Some(i) = self
                .notifications
                .iter()
                .position(|n| n.dismissed || n.resolved)
        {
            self.notifications.remove(i);
        }
        Some(nid)
    }
}

/// Bounds the resume command a hook helper may persist.
fn resume_too_large(r: &Resume) -> bool {
    r.program.len() > 4096
        || r.args.len() > 64
        || r.args.iter().map(String::len).sum::<usize>() > 16384
}
