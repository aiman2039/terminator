//! Shared agent identity/status presentation. One model backs workspace tabs,
//! terminal-strip tabs, split-pane headers, session rows, and agent rows so
//! identity, lifecycle, unread state, and diagnostics render consistently.
//!
//! Lifecycle status always comes from hook records. A verified live process
//! without hook state shows "Status unavailable", never "Working" or "Idle".
use terminator_core::{
    AGENT_PRESENCE_CAPABILITY, AgentState, Session, State,
    agents::{self, PresenceOutcome},
};

/// Status glyph per lifecycle state. The brand icon beside it identifies the
/// agent; this glyph only reports hook lifecycle.
#[must_use]
pub fn attention_status_icon(state: AgentState) -> &'static str {
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
pub struct AttentionCounts {
    pub input: usize,
    pub permission: usize,
    pub failed: usize,
}

impl AttentionCounts {
    #[must_use]
    pub fn waiting(self) -> usize {
        self.input + self.permission
    }
    #[must_use]
    pub fn total(self) -> usize {
        self.input + self.permission + self.failed
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.total() == 0
    }
}

#[must_use]
pub fn notice_pending(notice: &terminator_core::Notification, now: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= now
}

/// Aggregate pending attention over a set of terminal sessions.
#[must_use]
pub fn tab_attention(state: &State, session_ids: &[String], now: u64) -> AttentionCounts {
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
            AgentState::WaitingInput => counts.input += 1,
            AgentState::WaitingPermission => counts.permission += 1,
            AgentState::Failed => counts.failed += 1,
            _ => {}
        }
    }
    counts
}

/// Per-owner presence support for a session. Single-daemon states carry
/// capabilities on the snapshot; multi-owner states carry them per owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerSupport {
    pub supported: bool,
    pub available: bool,
}

#[must_use]
pub fn owner_support(state: &State, session: &Session) -> OwnerSupport {
    match state
        .generations
        .iter()
        .find(|health| health.owner.id == session.generation)
    {
        Some(health) => OwnerSupport {
            supported: health
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: health.error.is_none(),
        },
        None if state.generations.is_empty() => OwnerSupport {
            supported: state
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: true,
        },
        None => OwnerSupport {
            supported: false,
            available: false,
        },
    }
}

/// Complete presentation for one terminal session.
#[derive(Clone, Debug)]
pub struct AgentPresentation {
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
    /// Deferred tooltip inputs. Never contain command arguments.
    diagnostics: Option<Diagnostics>,
}

#[derive(Clone, Debug)]
struct Diagnostics {
    session: Session,
    presence: Option<agents::TerminalPresence>,
    detected: Vec<agents::DetectedAgent>,
    hook: Option<terminator_core::Agent>,
    support: OwnerSupport,
    hook_linked: bool,
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

/// Rebuilt on snapshot/local mutation, freshness changes, and snooze expiry.
/// Time alone does not rebuild stable presentations on every frame.
#[derive(Default)]
pub struct PresentationCache {
    entries: std::collections::HashMap<String, std::sync::Arc<AgentPresentation>>,
    fresh: Option<bool>,
    expires: u64,
}
impl PresentationCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn get(
        &mut self,
        state: &State,
        id: &str,
        now: u64,
        fresh: Option<bool>,
    ) -> std::sync::Arc<AgentPresentation> {
        if self.entries.is_empty() || self.fresh != fresh || now >= self.expires {
            self.entries = state
                .sessions
                .iter()
                .map(|s| {
                    (
                        s.id.clone(),
                        std::sync::Arc::new(present_session_with_freshness(
                            state, &s.id, now, fresh,
                        )),
                    )
                })
                .collect();
            self.fresh = fresh;
            self.expires = state
                .notifications
                .iter()
                .map(|n| n.snoozed_until)
                .chain(
                    state
                        .presence
                        .iter()
                        .filter(|_| fresh.is_none())
                        .map(|p| p.observed_at.saturating_add(agents::STALE_AFTER_SECS + 1)),
                )
                .filter(|at| *at > now)
                .min()
                .unwrap_or(u64::MAX);
        }
        self.entries.get(id).cloned().unwrap_or_else(|| {
            std::sync::Arc::new(present_session_with_freshness(state, id, now, fresh))
        })
    }
}

fn ago(timestamp: u64, now: u64) -> String {
    let age = now.saturating_sub(timestamp);
    if age < 90 {
        format!("{age}s ago")
    } else if age < 5400 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}

/// Present one terminal session. Callers must pass the current time; the
/// model never reads the clock itself so tests stay deterministic.
#[cfg(test)]
#[must_use]
pub fn present_session(state: &State, session_id: &str, now: u64) -> AgentPresentation {
    present_session_with_freshness(state, session_id, now, None)
}

fn present_session_with_freshness(
    state: &State,
    session_id: &str,
    now: u64,
    fresh: Option<bool>,
) -> AgentPresentation {
    let empty = || AgentPresentation {
        brand_icon: None,
        brand_label: None,
        detected_kinds: Vec::new(),
        verified: false,
        live: false,
        lifecycle: None,
        status_label: "Status unavailable".into(),
        status_icon: attention_status_icon(AgentState::Unknown),
        spin: false,
        unread: 0,
        attention: AttentionCounts::default(),
        diagnostics: None,
    };
    let Some(session) = state.sessions.iter().find(|s| s.id == session_id) else {
        return empty();
    };
    let support = owner_support(state, session);
    let presence = state.presence.iter().find(|p| p.session_id == session_id);
    let verified = session.lifecycle.live()
        && support.supported
        && support.available
        && presence.is_some_and(|p| p.outcome == PresenceOutcome::Verified)
        && fresh.unwrap_or_else(|| agents::presence_verified(presence, now));
    let detected: Vec<_> = if verified {
        presence.map(|p| p.agents.clone()).unwrap_or_default()
    } else {
        Vec::new()
    };
    let detected_kinds: Vec<String> = detected.iter().map(|a| a.kind.clone()).collect();
    let hook = agents::select_session_agent(&state.agents, session_id);
    let (brand_icon, brand_label) = if !detected.is_empty() {
        match agents::preferred_agent(&detected) {
            Some(agent) => (
                Some(agents::icon_key(&agent.kind)),
                Some(agents::display_name(&agent.kind).to_string()),
            ),
            None => (
                Some(agents::MULTIPLE_ICON),
                Some(format!("{} agents", detected.len())),
            ),
        }
    } else if let Some(hook) = hook {
        (
            Some(agents::icon_key(&hook.kind)),
            Some(agents::display_name(&hook.kind).to_string()),
        )
    } else {
        (None, None)
    };
    let lifecycle = hook.map(|a| a.state);
    let status_label = match lifecycle {
        Some(status) if status != AgentState::Unknown => status.label().to_string(),
        _ => "Status unavailable".to_string(),
    };
    let status_icon = attention_status_icon(lifecycle.unwrap_or(AgentState::Unknown));
    let hook_linked = match (hook, hook.and_then(|h| h.process.as_ref())) {
        (Some(current), Some(identity)) => detected
            .iter()
            .any(|a| a.kind == current.kind && a.process == *identity),
        _ => false,
    };
    let owned = [session_id.to_string()];
    let attention = tab_attention(state, &owned, now);
    let unread = state
        .notifications
        .iter()
        .filter(|n| n.session_id == session_id && !n.read && notice_pending(n, now))
        .count();
    let diagnostics = Some(Diagnostics {
        session: session.clone(),
        presence: presence.cloned(),
        detected: detected.clone(),
        hook: hook.cloned(),
        support,
        hook_linked,
    });
    AgentPresentation {
        brand_icon,
        brand_label,
        detected_kinds,
        verified,
        live: !detected.is_empty(),
        lifecycle,
        status_label,
        status_icon,
        spin: lifecycle == Some(AgentState::Running),
        unread,
        attention,
        diagnostics,
    }
}

#[allow(clippy::too_many_arguments)]
fn diagnostics_text(
    session: &Session,
    presence: Option<&terminator_core::agents::TerminalPresence>,
    detected: &[terminator_core::agents::DetectedAgent],
    hook: Option<&terminator_core::Agent>,
    support: OwnerSupport,
    verified: bool,
    hook_linked: bool,
    status_label: &str,
    unread: usize,
    now: u64,
) -> String {
    let mut lines = Vec::new();
    if !session.lifecycle.live() {
        lines.push("Session ended".into());
    } else if detected.is_empty() {
        if verified {
            lines.push(format!(
                "No live agent detected (verified {})",
                ago(presence.map(|p| p.observed_at).unwrap_or(now), now)
            ));
        } else if !support.available {
            lines.push("Presence unverified: session owner unavailable".into());
        } else if !support.supported {
            lines.push("Presence unverified: unsupported daemon".into());
        } else if presence.is_some_and(|p| p.outcome == PresenceOutcome::Unavailable) {
            lines.push("Presence unverified: detection unavailable".into());
        } else if let Some(observed) = presence.map(|p| p.observed_at) {
            lines.push(format!(
                "Presence unverified: stale observation ({})",
                ago(observed, now)
            ));
        } else {
            lines.push("Presence unverified: no observation yet".into());
        }
    } else {
        let names: Vec<_> = detected
            .iter()
            .map(|a| agents::display_name(&a.kind))
            .collect();
        let age = ago(presence.map(|p| p.observed_at).unwrap_or(now), now);
        if detected.len() == 1 && agents::preferred_agent(detected).is_some() {
            let how = if detected[0].foreground {
                "foreground"
            } else {
                "only agent"
            };
            lines.push(format!("{} live (verified {age}; {how})", names[0]));
        } else {
            lines.push(format!("{} live (verified {age})", names.join(", ")));
        }
    }
    lines.push(if detected.is_empty() {
        match hook {
            Some(_) => "Identity source: hook event".into(),
            None => "Identity source: none".into(),
        }
    } else {
        "Identity source: process inspection".into()
    });
    match hook {
        Some(h) => lines.push(format!(
            "Hook ({}): {} · {} · {}",
            agents::display_name(&h.kind),
            h.state.label(),
            if hook_linked {
                "linked to live process"
            } else {
                "last reported"
            },
            ago(h.updated, now),
        )),
        None => lines.push("Hook: none reported".into()),
    }
    lines.push(format!("Status: {status_label}"));
    if unread > 0 {
        lines.push(format!("Unread: {unread}"));
    }
    lines.push(session.cwd.display().to_string());
    lines.join("\n")
}

/// Sessions with verified live agents, for the All-live view.
#[cfg(test)]
#[must_use]
pub fn live_sessions(state: &State, now: u64) -> Vec<(&Session, Vec<String>)> {
    state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .filter_map(|s| {
            let presentation = present_session(state, &s.id, now);
            if presentation.live {
                Some((s, presentation.detected_kinds))
            } else {
                None
            }
        })
        .collect()
}

/// Live sessions with hook records but no verified live agent. Covers older
/// daemons, unavailable owners, stale observations, detection failures, and
/// agents that already exited: all render as presence unverified.
#[cfg(test)]
#[must_use]
pub fn unverified_sessions(state: &State, now: u64) -> Vec<&Session> {
    state
        .sessions
        .iter()
        .filter(|s| {
            s.lifecycle.live()
                && !present_session(state, &s.id, now).live
                && state.agents.iter().any(|a| a.session_id == s.id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::{
        Agent, Lifecycle, Notification, SessionKind, agents::ProcessIdentity, now,
    };

    fn session(id: &str) -> Session {
        Session {
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 1,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: Some(100),
            truncated: false,
            cwd_confirmed: true,
        }
    }

    fn capable(state: &mut State) {
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
    }

    fn observe(state: &mut State, kinds: &[&str], at: u64) {
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: kinds
                .iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: (*kind).into(),
                    process: ProcessIdentity {
                        pid: 200 + i as u32,
                        start_time: 500,
                    },
                    foreground: i == 0,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: at,
        });
    }

    fn hook(state: &mut State, kind: &str, lifecycle: AgentState, linked: bool) {
        state.agents.push(Agent {
            invocation_id: format!("agent-{kind}"),
            session_id: "s".into(),
            kind: kind.into(),
            provider_session_id: None,
            state: lifecycle,
            sequence: None,
            updated: now() - 3,
            resume: None,
            process: linked.then_some(ProcessIdentity {
                pid: 200,
                start_time: 500,
            }),
        });
    }

    #[test]
    fn cache_reuses_presentations_and_expires_snoozes_and_snapshot_health() {
        let mut state = State::default();
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
        state.sessions.push(session("s"));
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: PresenceOutcome::Verified,
            observed_at: 1,
        });
        let mut cache = PresentationCache::default();
        let first = cache.get(&state, "s", 100, Some(true));
        assert!(
            first.verified,
            "successful unchanged snapshots keep presence fresh"
        );
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &cache.get(&state, "s", 200, Some(true))
        ));
        assert!(!cache.get(&state, "s", 200, Some(false)).verified);
        assert!(cache.get(&state, "s", 201, Some(true)).verified);
        // A responsive daemon can explicitly expire a stalled inspector's data.
        state.presence[0].outcome = PresenceOutcome::Unavailable;
        cache.clear();
        assert!(!cache.get(&state, "s", 202, Some(true)).verified);
        state.presence[0].outcome = PresenceOutcome::Verified;
        state.presence[0].observed_at = 203;
        cache.clear();
        assert!(cache.get(&state, "s", 203, Some(true)).verified);
        state.notifications.push(Notification {
            id: "n".into(),
            session_id: "s".into(),
            invocation_id: "i".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 100,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 300,
        });
        cache.clear();
        assert_eq!(cache.get(&state, "s", 299, Some(true)).attention.input, 0);
        assert_eq!(cache.get(&state, "s", 300, Some(true)).attention.input, 1);
        state.sessions[0].lifecycle = Lifecycle::Ended;
        cache.clear();
        let ended = cache.get(&state, "s", 301, Some(true));
        assert!(!ended.verified);
        assert!(ended.attention.is_empty());
    }

    #[test]
    fn verified_detection_pairs_brand_with_hook_status() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.brand_label.as_deref(), Some("Codex"));
        assert!(presented.verified && presented.live);
        assert_eq!(presented.status_label, "Working");
        assert_eq!(presented.status_icon, "LoaderCircle");
        assert!(presented.spin);
        assert!(presented.diagnostics(now()).contains("process inspection"));
        assert!(
            presented
                .diagnostics(now())
                .contains("linked to live process")
        );
    }

    #[test]
    fn verified_presence_without_hooks_never_claims_working_or_idle() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["pi"], now());
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_label.as_deref(), Some("Pi"));
        assert_eq!(presented.status_label, "Status unavailable");
        assert!(!presented.spin);
        assert!(presented.diagnostics(now()).contains("Hook: none reported"));
    }

    #[test]
    fn hook_only_state_on_old_daemons_is_labeled_unverified() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "my-agent", AgentState::WaitingInput, false);
        let presented = present_session(&state, "s", now());
        assert!(!presented.verified && !presented.live);
        assert_eq!(presented.brand_icon, Some(agents::GENERIC_ICON));
        assert_eq!(presented.status_label, "Needs input");
        assert!(presented.diagnostics(now()).contains("unsupported daemon"));
        assert!(presented.diagnostics(now()).contains("last reported"));
    }

    #[test]
    fn multiple_agents_without_foreground_use_the_count_indicator() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: ["codex", "claude"]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: kind.into(),
                    process: ProcessIdentity {
                        pid: 200 + i as u32,
                        start_time: 500,
                    },
                    foreground: false,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: now(),
        });
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_icon, Some(agents::MULTIPLE_ICON));
        assert_eq!(presented.brand_label.as_deref(), Some("2 agents"));
        assert!(
            presented.diagnostics(now()).contains("Codex")
                && presented.diagnostics(now()).contains("Claude")
        );
    }

    #[test]
    fn stale_failed_and_unavailable_observations_stay_unverified() {
        for (outcome, age, needle) in [
            (PresenceOutcome::Verified, 30, "stale observation"),
            (PresenceOutcome::Unavailable, 0, "detection unavailable"),
        ] {
            let mut state = State::default();
            state.sessions.push(session("s"));
            capable(&mut state);
            state.presence.push(agents::TerminalPresence {
                session_id: "s".into(),
                generation: "g".into(),
                agents: vec![agents::DetectedAgent {
                    kind: "codex".into(),
                    process: ProcessIdentity {
                        pid: 200,
                        start_time: 500,
                    },
                    foreground: true,
                }],
                outcome,
                observed_at: now().saturating_sub(age),
            });
            hook(&mut state, "codex", AgentState::Running, true);
            let presented = present_session(&state, "s", now());
            assert!(!presented.verified && !presented.live, "{needle}");
            assert!(
                presented.diagnostics(now()).contains("last reported")
                    || presented.diagnostics(now()).contains("Hook: none reported"),
                "{}",
                presented.diagnostics(now())
            );
            assert!(
                presented.diagnostics(now()).contains(needle),
                "{}",
                presented.diagnostics(now())
            );
        }
    }

    #[test]
    fn muse_subagent_tool_after_stop_shows_completed() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state.agents.push(Agent {
            invocation_id: "lead".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e846-a59d-7da0-a7c8-3de6bf83174b".into()),
            state: AgentState::Completed,
            sequence: None,
            updated: 10,
            resume: None,
            process: None,
        });
        state.agents.push(Agent {
            invocation_id: "tool".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("ee1080b7-a350-4c2b-844b-ae1786b1b3c0".into()),
            state: AgentState::Running,
            sequence: None,
            updated: 50,
            resume: None,
            process: None,
        });
        let presented = present_session(&state, "s", 100);
        assert_eq!(presented.status_label, "Completed");
        assert!(!presented.spin);
    }

    #[test]
    fn unknown_lifecycle_shows_status_unavailable() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "codex", AgentState::Unknown, false);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.status_label, "Status unavailable");
        assert_eq!(presented.status_icon, "CircleQuestion");
    }

    #[test]
    fn attention_counts_keep_input_permission_and_failure_distinct() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        for (i, lifecycle) in [
            AgentState::WaitingInput,
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Failed,
            AgentState::Completed,
        ]
        .into_iter()
        .enumerate()
        {
            state.notifications.push(Notification {
                id: format!("n{i}"),
                session_id: "s".into(),
                invocation_id: "a".into(),
                request_id: None,
                state: lifecycle,
                summary: String::new(),
                details: String::new(),
                created: i as u64,
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            });
        }
        // Dismissed, resolved, and snoozed notices never count.
        state.notifications[4].resolved = true;
        state.notifications.push(Notification {
            id: "snoozed".into(),
            session_id: "s".into(),
            invocation_id: "a".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 9,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: now() + 600,
        });
        let counts = tab_attention(&state, &["s".into(), "other".into()], now());
        assert_eq!(
            counts,
            AttentionCounts {
                input: 2,
                permission: 1,
                failed: 1,
            }
        );
        assert_eq!(present_session(&state, "s", now()).unread, 4);
        assert!(tab_attention(&state, &["other".into()], now()).is_empty());
    }

    #[test]
    fn live_and_unverified_session_lists_partition_hook_activity() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        let mut ended = session("ended");
        ended.lifecycle = Lifecycle::Ended;
        state.sessions.push(ended);
        state.sessions.push(session("plain"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        state.agents.push(Agent {
            invocation_id: "hook-only".into(),
            session_id: "plain".into(),
            kind: "claude".into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            sequence: None,
            updated: now(),
            resume: None,
            process: None,
        });
        let live = live_sessions(&state, now());
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0.id, "s");
        let unverified = unverified_sessions(&state, now());
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].id, "plain");
    }
}
