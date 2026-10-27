use super::tests::{ended_session, event, setup};
use super::*;

#[test]
fn legacy_snapshots_do_not_advertise_new_daemon_features() {
    let legacy: State = serde_json::from_value(serde_json::json!({"revision":7})).unwrap();
    assert!(legacy.capabilities.is_empty());
    assert!(legacy.daemon_version.is_none());
    let new = State {
        capabilities: vec![NVIM_REVIEW_CAPABILITY.into()],
        ..State::default()
    };
    let encoded = serde_json::to_value(&new).unwrap();
    #[derive(Deserialize)]
    struct LegacyState {
        revision: u64,
    }
    assert_eq!(
        serde_json::from_value::<LegacyState>(encoded.clone())
            .unwrap()
            .revision,
        0
    );
    assert_eq!(
        serde_json::from_value::<State>(encoded)
            .unwrap()
            .capabilities,
        new.capabilities
    );
}

#[test]
fn restarting_changes_hint_even_if_revision_is_equal() {
    let mut state = State::default();
    state.recover();
    let previous = state.snapshot_hint();
    state.recover();
    state.revision = previous.revision;
    assert_ne!(previous, state.snapshot_hint());
}

fn presence_for(session: &str, kind: &str, pid: u32, start: u64) -> State {
    let mut state = setup();
    state.presence.push(agents::TerminalPresence {
        session_id: session.into(),
        generation: state.sessions[0].generation.clone(),
        agents: vec![agents::DetectedAgent {
            kind: kind.into(),
            process: agents::ProcessIdentity {
                pid,
                start_time: start,
            },
            foreground: true,
        }],
        outcome: agents::PresenceOutcome::Verified,
        observed_at: now(),
    });
    state
}

#[test]
fn hook_process_identity_links_only_to_a_verified_live_observation() {
    let mut state = presence_for("s", "codex", 4242, 777);
    let mut linked = event(1, AgentState::Running);
    linked.agent_kind = "codex".into();
    linked.process = Some(agents::ProcessIdentity {
        pid: 4242,
        start_time: 777,
    });
    state.apply_hook(linked).unwrap();
    assert_eq!(
        state.agents[0].process,
        Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777
        })
    );
    // Wrong PID, recycled start time, and wrong kind never link.
    for (pid, start, kind) in [
        (9999, 777, "codex"),
        (4242, 778, "codex"),
        (4242, 777, "claude"),
    ] {
        let mut state = presence_for("s", "codex", 4242, 777);
        let mut e = event(1, AgentState::Running);
        e.agent_kind = kind.into();
        e.event_id = format!("e-{pid}-{start}-{kind}");
        e.process = Some(agents::ProcessIdentity {
            pid,
            start_time: start,
        });
        state.apply_hook(e).unwrap();
        assert!(
            state.agents[0].process.is_none(),
            "unverified identity must not link: {pid}/{start}/{kind}"
        );
        // ... but lifecycle delivery is preserved.
        assert_eq!(state.agents[0].state, AgentState::Running);
    }
    // Stale observations cannot verify new links.
    let mut state = presence_for("s", "codex", 4242, 777);
    state.presence[0].observed_at = now().saturating_sub(30);
    let mut e = event(1, AgentState::Running);
    e.agent_kind = "codex".into();
    e.process = Some(agents::ProcessIdentity {
        pid: 4242,
        start_time: 777,
    });
    state.apply_hook(e).unwrap();
    assert!(state.agents[0].process.is_none());
}

#[test]
fn hook_without_process_identity_preserves_existing_delivery() {
    let mut state = presence_for("s", "codex", 4242, 777);
    let delivered = state
        .apply_hook(event(1, AgentState::WaitingInput))
        .unwrap();
    assert!(delivered.is_some());
    assert!(state.agents[0].process.is_none());
    assert_eq!(state.agents[0].state, AgentState::WaitingInput);
    assert_eq!(state.notifications.len(), 1);
}

#[test]
fn new_invocation_cannot_inherit_an_earlier_invocations_process_link() {
    let mut state = presence_for("s", "codex", 4242, 777);
    let mut first = event(1, AgentState::Running);
    first.agent_kind = "codex".into();
    first.agent_invocation_id = "first".into();
    first.process = Some(agents::ProcessIdentity {
        pid: 4242,
        start_time: 777,
    });
    state.apply_hook(first).unwrap();
    assert!(state.agents[0].process.is_some());
    let mut second = event(2, AgentState::Running);
    second.agent_kind = "codex".into();
    second.agent_invocation_id = "second".into();
    second.event_id = "e-second".into();
    second.process = Some(agents::ProcessIdentity {
        pid: 4242,
        start_time: 777,
    });
    // The detected process left; the observation is empty now.
    state.presence[0].agents.clear();
    state.apply_hook(second).unwrap();
    assert_eq!(state.agents.len(), 2);
    assert!(state.agents[1].process.is_none());
}

#[test]
fn presence_observations_never_emit_lifecycle_events_or_notifications() {
    let state = presence_for("s", "codex", 4242, 777);
    assert!(state.agents.is_empty());
    assert!(state.notifications.is_empty());
    assert!(state.recent_events.is_empty());
}

#[test]
fn presence_fields_are_additive_and_old_snapshots_parse_cleanly() {
    let legacy: State = serde_json::from_value(serde_json::json!({
        "revision": 3,
        "agents": [{"invocation_id": "a", "session_id": "s", "kind": "codex",
                    "provider_session_id": null, "state": "running",
                    "sequence": null, "updated": 1, "resume": null}],
    }))
    .unwrap();
    assert!(legacy.presence.is_empty());
    assert!(legacy.agents[0].process.is_none());
    let legacy_event: HookEvent = serde_json::from_value(serde_json::json!({
        "protocol_version": 1, "event_id": "e", "terminal_session_id": "s",
        "agent_invocation_id": "a", "agent_kind": "codex",
        "provider_session_id": null, "state": "running", "request_id": null,
        "sequence": null, "summary": "hi", "details": "", "resume": null,
    }))
    .unwrap();
    assert!(legacy_event.process.is_none());
    // New snapshots round-trip through the additive fields.
    let state = presence_for("s", "codex", 4242, 777);
    let encoded = serde_json::to_value(&state).unwrap();
    assert_eq!(encoded["presence"][0]["agents"][0]["kind"], "codex");
    let restored: State = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored.presence, state.presence);
}

#[test]
fn recovery_clears_presence_and_process_links_without_relaunching() {
    let mut state = presence_for("s", "codex", 4242, 777);
    let mut e = event(1, AgentState::Running);
    e.agent_kind = "codex".into();
    e.process = Some(agents::ProcessIdentity {
        pid: 4242,
        start_time: 777,
    });
    state.apply_hook(e).unwrap();
    assert!(state.agents[0].process.is_some());
    state.recover();
    assert!(state.presence.is_empty());
    assert!(state.agents[0].process.is_none());
    assert_eq!(state.sessions[0].lifecycle, Lifecycle::Interrupted);
}

#[test]
fn prune_drops_presence_for_removed_sessions() {
    let mut s = State::default();
    s.sessions.push(ended_session("gone", Lifecycle::Ended));
    s.presence.push(agents::TerminalPresence {
        session_id: "gone".into(),
        generation: "g".into(),
        agents: Vec::new(),
        outcome: agents::PresenceOutcome::Verified,
        observed_at: now(),
    });
    s.prune_non_resumable_ended();
    assert!(s.presence.is_empty());
}
