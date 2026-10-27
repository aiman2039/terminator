use super::*;

#[test]
#[cfg(unix)]
fn shell_preference_skips_missing_and_nonexecutable_files() {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![dir.path().to_path_buf()];
    let choose = || {
        ["zsh", "bash", "sh"]
            .into_iter()
            .find_map(|name| find_executable_in(name, &paths))
    };
    assert!(choose().is_none());
    for name in ["sh", "bash", "zsh"] {
        let p = dir.path().join(name);
        fs::write(&p, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(choose(), Some(p));
    }
    fs::set_permissions(dir.path().join("zsh"), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(choose(), Some(dir.path().join("bash")));
    fs::remove_file(dir.path().join("bash")).unwrap();
    assert_eq!(choose(), Some(dir.path().join("sh")));
}

#[test]
fn bundled_names_already_carry_the_platform_suffix() {
    for name in bundled_exes() {
        assert_eq!(exe_name(&name), name);
        assert_eq!(
            sibling_exe(Path::new("/tmp/terminator"), &name),
            Path::new("/tmp/terminator").with_file_name(&name)
        );
    }
}

#[test]
#[cfg(windows)]
fn windows_lookup_finds_bare_names_with_executable_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![dir.path().to_path_buf()];
    assert!(find_executable_in("nvim", &paths).is_none());
    std::fs::write(dir.path().join("nvim.exe"), b"fixture").unwrap();
    assert_eq!(
        find_executable_in("nvim", &paths).map(|path| path.canonicalize().unwrap()),
        Some(dir.path().join("nvim.exe").canonicalize().unwrap())
    );
    assert_eq!(
        find_executable_in("nvim.exe", &paths),
        Some(dir.path().join("nvim.exe"))
    );
}

/// Keep Unix socket paths short; Windows uses the system temporary directory.
pub(crate) fn tempdir(prefix: &str) -> tempfile::TempDir {
    #[cfg(unix)]
    let root = PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let root = std::env::temp_dir();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .unwrap()
}

pub(crate) fn setup() -> State {
    let mut s = State::default();
    s.sessions.push(Session {
        remote: None,
        review: false,
        id: "s".into(),
        project_id: "p".into(),
        label: "shell".into(),
        cwd: "/tmp".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Running,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: id(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    });
    s
}

#[test]
fn terminal_notifications_never_create_or_change_agents_and_are_bounded() {
    let mut state = setup();
    state
        .terminal_notice("s", "Terminal title", "permission needed")
        .unwrap();
    assert!(state.agents.is_empty());
    assert!(state.notifications.is_empty());
    assert!(
        state
            .terminal_notice("s", "Terminal title", "permission needed")
            .unwrap()
            .is_none()
    );
    assert!(state.terminal_notice("missing", "test", "test").is_err());
    for i in 0..200 {
        state
            .terminal_notice("s", &format!("{i}"), "hello")
            .unwrap();
    }
    assert_eq!(state.terminal_notices.len(), 128);
}
pub(crate) fn event(n: u64, state: AgentState) -> HookEvent {
    HookEvent {
        protocol_version: 1,
        event_id: format!("e{n}"),
        terminal_session_id: "s".into(),
        agent_invocation_id: "a".into(),
        agent_kind: "custom".into(),
        provider_session_id: None,
        state,
        request_id: Some("req".into()),
        sequence: Some(n),
        summary: "Need input".into(),
        details: String::new(),
        resume: None,
        process: None,
    }
}

#[test]
fn ntfy_eligible_notifications_suppress_duplicate_and_stale_hooks() {
    let mut state = setup();
    state.settings.ntfy_enabled = true;
    state.settings.ntfy_channel = "fixture".into();
    assert!(
        state
            .apply_hook(event(2, AgentState::WaitingInput))
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .apply_hook(event(2, AgentState::WaitingInput))
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .apply_hook(event(1, AgentState::Completed))
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .apply_hook(event(3, AgentState::WaitingInput))
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .apply_hook(event(4, AgentState::Completed))
            .unwrap()
            .is_some()
    );
}

#[test]
fn dismissal_does_not_resume_agent() {
    let mut s = setup();
    s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
    s.focus("s");
    assert!(s.notifications[0].dismissed);
    assert_eq!(s.agents[0].state, AgentState::WaitingInput);
    s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
    assert_eq!(s.notifications.len(), 1);
}

#[test]
fn stale_events_and_other_invocations() {
    let mut s = setup();
    s.apply_hook(event(3, AgentState::Running)).unwrap();
    s.apply_hook(event(2, AgentState::WaitingPermission))
        .unwrap();
    assert!(s.notifications.is_empty());
    let mut e = event(4, AgentState::Completed);
    e.agent_invocation_id = "old-agent".into();
    s.apply_hook(e).unwrap();
    assert_eq!(s.agents[0].state, AgentState::Running);
}

#[test]
fn recovery_does_not_relaunch() {
    let mut s = setup();
    s.recover();
    assert_eq!(s.sessions[0].lifecycle, Lifecycle::Interrupted);
    assert!(s.sessions[0].pid.is_none());
}

#[test]
fn recovery_preserves_history_loss_but_discards_previous_runtime_health() {
    let mut state = setup();
    state.sessions[0].truncated = true;
    state.degraded = Some("Output storage queue saturated; some history was not saved".into());
    state.daemon_executable = Some("/removed/daemon".into());
    state.attachment_helper_executable = Some("/removed/helper".into());
    state.attachment_helper_available = Some(false);
    state.recover();
    assert!(state.sessions[0].truncated);
    assert!(state.degraded.is_none());
    assert!(state.daemon_executable.is_none());
    assert!(state.attachment_helper_executable.is_none());
    assert!(state.attachment_helper_available.is_none());
}

#[test]
fn shell_quote_is_literal() {
    assert_eq!(quote("a'b$(x)"), "'a'\\''b$(x)'");
}

fn agent_for(session: &str, resume: bool) -> Agent {
    Agent {
        invocation_id: format!("agent-{session}"),
        session_id: session.into(),
        kind: "codex".into(),
        provider_session_id: resume.then(|| "provider-1".into()),
        state: AgentState::Stopped,
        sequence: None,
        updated: 0,
        resume: resume.then(|| Resume {
            program: "codex".into(),
            args: vec!["resume".into(), "provider-1".into()],
        }),
        process: None,
    }
}

#[test]
fn prune_keeps_only_live_or_resumable_ended_sessions() {
    let mut s = State::default();
    s.sessions
        .push(ended_session("plain-ended", Lifecycle::Ended));
    s.sessions
        .push(ended_session("plain-interrupted", Lifecycle::Interrupted));
    s.sessions
        .push(ended_session("resumable-ended", Lifecycle::Ended));
    s.sessions
        .push(ended_session("agent-without-resume", Lifecycle::Ended));
    let mut live = ended_session("live", Lifecycle::Running);
    live.lifecycle = Lifecycle::Running;
    s.sessions.push(live);
    s.agents.push(agent_for("resumable-ended", true));
    s.agents.push(agent_for("agent-without-resume", false));
    s.notifications.push(Notification {
        id: "n".into(),
        session_id: "plain-ended".into(),
        invocation_id: "x".into(),
        request_id: None,
        state: AgentState::Stopped,
        summary: String::new(),
        details: String::new(),
        created: 0,
        read: false,
        dismissed: false,
        resolved: false,
        snoozed_until: 0,
    });
    assert!(s.session_has_resume("resumable-ended"));
    assert!(!s.session_has_resume("plain-ended"));
    assert!(!s.session_has_resume("agent-without-resume"));
    let mut removed = s.prune_non_resumable_ended();
    removed.sort();
    assert_eq!(
        removed,
        vec![
            "agent-without-resume".to_string(),
            "plain-ended".to_string(),
            "plain-interrupted".to_string()
        ]
    );
    let kept: Vec<_> = s
        .sessions
        .iter()
        .map(|session| session.id.clone())
        .collect();
    assert!(kept.contains(&"resumable-ended".to_string()));
    assert!(kept.contains(&"live".to_string()));
    assert!(s.notifications.is_empty());
    assert!(s.agents.iter().all(|a| a.session_id != "plain-ended"));
}

#[test]
fn compact_history_bounds_records_and_keeps_pending_attention() {
    fn notice(id: String, settled: bool) -> Notification {
        Notification {
            id,
            session_id: "s".into(),
            invocation_id: "x".into(),
            request_id: None,
            state: AgentState::Completed,
            summary: String::new(),
            details: String::new(),
            created: 0,
            read: settled,
            dismissed: settled,
            resolved: settled,
            snoozed_until: 0,
        }
    }
    let mut s = State::default();
    s.notifications.push(notice("pending".into(), false));
    for i in 0..(MAX_NOTIFICATIONS + 200) {
        s.notifications.push(notice(format!("n{i}"), true));
    }
    // Live and resumable agents must survive even when older than the cap.
    let mut running = agent_for("live-agent", false);
    running.state = AgentState::Running;
    s.agents.push(running);
    s.agents.push(agent_for("resumable-agent", true));
    for i in 0..(MAX_AGENTS + 200) {
        s.agents.push(agent_for(&format!("stopped-{i}"), false));
    }
    s.recent_events.push("event".into());
    s.compact_history();
    assert!(s.recent_events.is_empty());
    assert!(s.notifications.iter().any(|n| n.id == "pending"));
    assert!(s.notifications.len() <= MAX_NOTIFICATIONS + 1);
    assert!(s.agents.iter().any(|a| a.session_id == "live-agent"));
    assert!(s.agents.iter().any(|a| a.session_id == "resumable-agent"));
    assert!(s.agents.len() <= MAX_AGENTS + 2);
}

pub(crate) fn ended_session(id: &str, lifecycle: Lifecycle) -> Session {
    Session {
        remote: None,
        review: false,
        id: id.into(),
        project_id: "p".into(),
        label: id.into(),
        cwd: "/tmp".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "g".into(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    }
}
