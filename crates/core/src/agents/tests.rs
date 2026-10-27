use super::*;

fn proc(pid: u32, parent: Option<u32>, exe: &str) -> ProcView {
    ProcView {
        pid,
        parent,
        start_time: 1000 + u64::from(pid),
        exe_name: exe.into(),
        argv: vec![format!("/usr/local/bin/{exe}")],
        live: true,
    }
}

#[test]
fn cursor_node_launcher_matches_agent_alias_without_matching_generic_scripts() {
    let entry = std::env::temp_dir().join("cursor-agent/versions/2026.10.19-6bbf2c4/index.js");
    let entry = entry.to_string_lossy();
    for invoked in ["node", "agent", "/Users/test/.local/bin/agent"] {
        assert_eq!(
            match_agent("node", &[invoked, "--use-system-ca", &entry]),
            Some("cursor")
        );
    }
    for script in [
        "/tmp/index.js",
        "/tmp/not-cursor-agent/versions/2026.10.19/index.js",
        "/tmp/cursor-agent/versions/not-a-version/index.js",
        "cursor-agent/versions/2026.10.19/index.js",
    ] {
        assert_eq!(match_agent("node", &["agent", script]), None);
    }
    assert_eq!(
        match_agent("node", &["node", "/tmp/index.js", &entry]),
        None
    );
    assert_eq!(match_agent("agent", &["agent"]), None);
    assert_eq!(match_agent("python", &["python", &entry]), None);
}

#[test]
fn catalog_matchers_cover_exact_executables() {
    for (kind, exe) in [
        ("codex", "codex"),
        ("claude", "claude"),
        ("pi", "pi"),
        ("opencode", "opencode"),
        ("grok", "grok"),
        ("muse", "muse"),
        ("cursor", "cursor-agent"),
    ] {
        assert_eq!(match_agent(exe, &[exe]), Some(kind), "{exe}");
        assert_eq!(
            match_agent(&format!("/usr/local/bin/{exe}"), &[exe]),
            Some(kind),
            "path {exe}"
        );
    }
    assert_eq!(CATALOG.len(), 7);
    // Bare `agent` is also Grok's PATH shim. Identity stays on `cursor-agent`.
    assert_eq!(match_agent("agent", &["agent"]), None);
    assert_eq!(
        match_agent(
            "cursor-agent-2026.02.27-e7d2ef6",
            &["/Users/test/.local/bin/agent"]
        ),
        Some("cursor")
    );
    assert_eq!(
        match_agent("bash", &["bash", "/Users/test/.local/bin/cursor-agent"]),
        Some("cursor")
    );
}

#[test]
fn wrappers_match_exact_names_and_known_entrypoints() {
    assert_eq!(match_agent("codex-bin", &["codex-bin"]), Some("codex"));
    assert_eq!(
        match_agent("node", &["node", "/opt/claude.js"]),
        Some("claude")
    );
    assert_eq!(
        match_agent("bun", &["bun", "--smol", "dist/pi-agent.js"]),
        Some("pi")
    );
    assert_eq!(
        match_agent("node", &["node", "/opt/opencode.mjs"]),
        Some("opencode")
    );
}

#[test]
fn versioned_launcher_binaries_match_but_helpers_do_not() {
    assert_eq!(
        match_agent(
            "muse-bin-1.4.1-R4503.1",
            &["/Users/test/.local/bin/muse-bin-1.4.1-R4503.1"]
        ),
        Some("muse")
    );
    assert_eq!(
        match_agent(
            "/Users/test/.local/bin/muse-bin-1.4.1-R4503.1",
            &["muse-bin-1.4.1-R4503.1"]
        ),
        Some("muse")
    );
    assert_eq!(match_agent("muse-1.4.1", &["muse-1.4.1"]), Some("muse"));
    assert_eq!(
        match_agent("grok-1.0.44-macos-aarch64", &["/Users/test/.grok/bin/grok"]),
        Some("grok")
    );
    // Claude's symlink resolves the exe to a bare version number, so the
    // `argv[0]` invocation carries the identity.
    assert_eq!(
        match_agent("2.1.283", &["/Users/test/.local/bin/claude"]),
        Some("claude")
    );
    assert_eq!(match_agent("2.1.283", &["claude"]), Some("claude"));
    // A bare version invoked directly carries no agent identity.
    assert_eq!(match_agent("2.1.283", &["2.1.283"]), None);
    assert_eq!(
        match_agent("codex-helper", &["codex-helper"]),
        None,
        "helper suffix still rejected"
    );
    assert_eq!(match_agent("muse-beta", &["muse-beta"]), None);
    let procs = vec![
        proc(100, None, "zsh"),
        proc(200, Some(100), "muse-bin-1.4.1-R4503.1"),
    ];
    let found = detect_agents(&procs, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert_eq!(found.agents.len(), 1);
    assert_eq!(found.agents[0].kind, "muse");
    let resolved = vec![
        proc(100, None, "zsh"),
        ProcView {
            pid: 200,
            parent: Some(100),
            start_time: 1300,
            exe_name: "2.1.283".into(),
            argv: vec!["/Users/test/.local/bin/claude".into()],
            live: true,
        },
    ];
    let found = detect_agents(&resolved, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert_eq!(found.agents.len(), 1);
    assert_eq!(found.agents[0].kind, "claude");
}

#[test]
fn shell_script_launcher_matches_before_the_real_binary_execs() {
    assert_eq!(
        match_agent("bash", &["bash", "/Users/test/.local/bin/muse"]),
        Some("muse")
    );
    assert_eq!(
        match_agent(
            "/bin/bash",
            &["bash", "/Users/test/.local/bin/muse", "--version"]
        ),
        Some("muse")
    );
    assert_eq!(match_agent("zsh", &["/bin/zsh", "-l", "-i"]), None);
    assert_eq!(match_agent("bash", &["bash", "-c", "echo muse"]), None);
    let procs = vec![
        proc(100, None, "zsh"),
        ProcView {
            pid: 200,
            parent: Some(100),
            start_time: 1200,
            exe_name: "bash".into(),
            argv: vec!["bash".into(), "/Users/test/.local/bin/muse".into()],
            live: true,
        },
    ];
    let found = detect_agents(&procs, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert_eq!(found.agents.len(), 1);
    assert_eq!(found.agents[0].kind, "muse");
}

#[test]
fn arbitrary_arguments_titles_and_directories_never_match() {
    // Agent names in arguments, flags, or script bodies are not identity.
    assert_eq!(match_agent("sh", &["sh", "-c", "codex resume abc"]), None);
    assert_eq!(match_agent("node", &["node", "server.js", "codex"]), None);
    assert_eq!(
        match_agent("node", &["node", "/home/user/my-codex-server.js"]),
        None
    );
    assert_eq!(match_agent("python3", &["python3", "claude.py"]), None);
    assert_eq!(match_agent("codex-helper", &["codex-helper"]), None);
    assert_eq!(match_agent("mycodex", &["mycodex"]), None);
    // Only the first non-flag argument is examined: a known entrypoint
    // later on the command line never matches.
    assert_eq!(
        match_agent("node", &["node", "server.js", "claude.js"]),
        None
    );
}

#[test]
fn child_tools_keep_the_root_identity_and_nested_agents_are_ignored() {
    let procs = vec![
        proc(100, None, "zsh"),
        proc(200, Some(100), "codex"),
        // Child tools of the agent, including a nested agent binary.
        proc(201, Some(200), "rg"),
        proc(202, Some(200), "claude"),
        proc(203, Some(202), "node"),
    ];
    let found = detect_agents(&procs, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert_eq!(found.agents.len(), 1);
    assert_eq!(found.agents[0].kind, "codex");
    assert_eq!(found.agents[0].process.pid, 200);
}

#[test]
fn independently_launched_sibling_agents_each_produce_an_entry() {
    let procs = vec![
        proc(100, None, "zsh"),
        proc(200, Some(100), "codex"),
        proc(300, Some(100), "claude"),
    ];
    let found = detect_agents(&procs, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert_eq!(found.agents.len(), 2);
    // No foreground process: no single preferred agent.
    assert!(preferred_agent(&found.agents).is_none());
}

#[test]
fn foreground_association_prefers_one_agent_for_the_icon() {
    let procs = vec![
        proc(100, None, "zsh"),
        proc(200, Some(100), "codex"),
        proc(300, Some(100), "claude"),
    ];
    let found = detect_agents(&procs, 100, 1100, Some(300));
    assert_eq!(preferred_agent(&found.agents).unwrap().kind, "claude");
    let single = detect_agents(&procs[..2], 100, 1100, None);
    assert_eq!(preferred_agent(&single.agents).unwrap().kind, "codex");
}

#[test]
fn pid_reuse_and_missing_shells_cannot_claim_verified_presence() {
    let procs = vec![proc(100, None, "zsh"), proc(200, Some(100), "codex")];
    // Same PID, recycled start time: shell identity changed.
    let stale = detect_agents(&procs, 100, 9999, None);
    assert_eq!(stale.outcome, PresenceOutcome::Unavailable);
    assert!(stale.agents.is_empty());
    // Shell gone entirely.
    let gone = detect_agents(&procs, 999, 1999, None);
    assert_eq!(gone.outcome, PresenceOutcome::Unavailable);
}

#[test]
fn exit_removes_live_identity_on_the_next_successful_inspection() {
    let before = vec![proc(100, None, "zsh"), proc(200, Some(100), "pi")];
    assert_eq!(detect_agents(&before, 100, 1100, None).agents.len(), 1);
    let after = vec![proc(100, None, "zsh")];
    let found = detect_agents(&after, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert!(found.agents.is_empty());
}

#[test]
fn terminated_but_unreaped_processes_have_already_exited() {
    let mut zombie = proc(200, Some(100), "codex");
    zombie.live = false;
    let procs = vec![proc(100, None, "zsh"), zombie];
    let found = detect_agents(&procs, 100, 1100, None);
    assert_eq!(found.outcome, PresenceOutcome::Verified);
    assert!(found.agents.is_empty());
    let mut dead_shell = proc(100, None, "zsh");
    dead_shell.live = false;
    let gone = detect_agents(&[dead_shell], 100, 1100, None);
    assert_eq!(gone.outcome, PresenceOutcome::Unavailable);
}

#[test]
fn observations_older_than_five_seconds_are_not_verified_presence() {
    let presence = TerminalPresence {
        session_id: "s".into(),
        generation: "g".into(),
        agents: vec![DetectedAgent {
            kind: "codex".into(),
            process: ProcessIdentity {
                pid: 1,
                start_time: 1,
            },
            foreground: false,
        }],
        outcome: PresenceOutcome::Verified,
        observed_at: 100,
    };
    assert!(presence_verified(Some(&presence), 105));
    assert!(!presence_verified(Some(&presence), 106));
    let unavailable = TerminalPresence {
        outcome: PresenceOutcome::Unavailable,
        observed_at: 105,
        ..presence.clone()
    };
    assert!(!presence_verified(Some(&unavailable), 105));
    assert!(!presence_verified(None, 105));
}

#[test]
fn muse_tool_records_do_not_outrank_the_conversation() {
    let agents = vec![
        crate::Agent {
            invocation_id: "lead".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e846-a59d-7da0-a7c8-3de6bf83174b".into()),
            state: crate::AgentState::Completed,
            sequence: None,
            updated: 10,
            resume: None,
            process: None,
        },
        crate::Agent {
            invocation_id: "tool".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("ee1080b7-a350-4c2b-844b-ae1786b1b3c0".into()),
            state: crate::AgentState::Running,
            sequence: None,
            updated: 50,
            resume: None,
            process: None,
        },
    ];
    let selected = select_session_agent(&agents, "s").unwrap();
    assert_eq!(selected.invocation_id, "lead");
    assert_eq!(selected.state, crate::AgentState::Completed);
}

#[test]
fn muse_new_conversation_replaces_a_finished_one() {
    let agents = vec![
        crate::Agent {
            invocation_id: "old".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e28b-93b7-79e2-b68a-afafe2b9146b".into()),
            state: crate::AgentState::Completed,
            sequence: None,
            updated: 10,
            resume: None,
            process: None,
        },
        crate::Agent {
            invocation_id: "new".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e8b4-2220-7dc3-8096-aaed0ec12cf8".into()),
            state: crate::AgentState::Running,
            sequence: None,
            updated: 40,
            resume: None,
            process: None,
        },
        crate::Agent {
            invocation_id: "tool".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("a38cd0bf-3185-4ffe-9a75-d230070f9fba".into()),
            state: crate::AgentState::Running,
            sequence: None,
            updated: 50,
            resume: None,
            process: None,
        },
    ];
    let selected = select_session_agent(&agents, "s").unwrap();
    assert_eq!(selected.invocation_id, "new");
    assert_eq!(selected.state, crate::AgentState::Running);
}

#[test]
fn custom_kinds_fall_back_to_the_generic_icon() {
    assert_eq!(icon_key("codex"), "AgentCodex");
    assert_eq!(icon_key("my-agent"), GENERIC_ICON);
    assert_eq!(display_name("my-agent"), "my-agent");
    assert_eq!(display_name("muse"), "Muse Code");
}
