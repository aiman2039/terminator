//! Gemini CLI, GitHub Copilot CLI, Factory Droid and Qwen Code installers.
use super::*;

fn run(home: &Path, kind: &str, remove: bool) -> PathBuf {
    install_at(InstallAtInput {
        home,
        kind,
        helper: Path::new("/tmp/space dir/hook"),
        remove,
        paths: &Paths::at(home.join("data")),
    })
    .unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn new_agent_installers_keep_user_settings_and_hooks_and_are_idempotent() {
    for kind in ["gemini", "droid", "qwen"] {
        let home = tempfile::tempdir().unwrap();
        let path = config_path(home.path(), kind).unwrap();
        atomic_write(
            &path,
            br#"{"theme":"dark","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
        )
        .unwrap();
        run(home.path(), kind, false);
        let once = fs::read_to_string(&path).unwrap();
        assert!(once.contains(MARKER), "{kind}");
        run(home.path(), kind, false);
        assert_eq!(once, fs::read_to_string(&path).unwrap(), "{kind}");
        assert!(installed(home.path(), kind));
        run(home.path(), kind, true);
        let after = read_json(&path);
        assert_eq!(after["theme"], "dark", "{kind}");
        assert!(after.to_string().contains("echo mine"), "{kind}");
        assert!(!after.to_string().contains(MARKER), "{kind}");
        let backups = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("terminator-backup")
            })
            .count();
        assert!(backups >= 1, "{kind} keeps a backup");
    }
}

#[test]
fn gemini_hooks_print_json_and_use_millisecond_timeouts() {
    let home = tempfile::tempdir().unwrap();
    let path = run(home.path(), "gemini", false);
    assert!(path.ends_with(".gemini/settings.json"));
    let doc = read_json(&path);
    for event in [
        "SessionStart",
        "BeforeAgent",
        "AfterTool",
        "Notification",
        "AfterAgent",
        "SessionEnd",
    ] {
        let hook = &doc["hooks"][event][0]["hooks"][0];
        assert_eq!(hook["timeout"], 5000, "{event}");
        let command = hook["command"].as_str().unwrap();
        assert!(command.contains("event gemini"), "{event}");
        assert!(
            command.contains("printf '%s\\n' '{}'"),
            "{event}: Gemini parses stdout as JSON"
        );
    }
    assert!(
        doc["hooks"].get("PreToolUse").is_none(),
        "Claude names are not Gemini events"
    );
}

#[test]
fn copilot_hooks_use_vs_code_event_names_in_their_own_file() {
    let home = tempfile::tempdir().unwrap();
    let path = config_path(home.path(), "copilot").unwrap();
    atomic_write(
        &path,
        br#"{"version":1,"hooks":{"sessionStart":[{"type":"command","bash":"echo mine"}]}}"#,
    )
    .unwrap();
    run(home.path(), "copilot", false);
    let doc = read_json(&path);
    assert_eq!(doc["version"], 1);
    for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
        let hook = &doc["hooks"][event][0];
        assert_eq!(hook["type"], "command");
        assert!(
            hook["bash"].as_str().unwrap().contains("event copilot"),
            "{event}"
        );
    }
    run(home.path(), "copilot", true);
    let doc = read_json(&path);
    assert_eq!(doc["hooks"]["sessionStart"][0]["bash"], "echo mine");
    assert!(doc["hooks"].get("Stop").is_none());
    atomic_write(&path, br#"{"version":2,"hooks":{}}"#).unwrap();
    let refused = install_at(InstallAtInput {
        home: home.path(),
        kind: "copilot",
        helper: Path::new("/tmp/hook"),
        remove: false,
        paths: &Paths::at(home.path().join("data")),
    });
    assert!(refused.is_err(), "an unknown schema is left untouched");
}

#[test]
fn new_agent_removal_without_config_creates_no_files() {
    for kind in ["gemini", "copilot", "droid", "qwen"] {
        let home = tempfile::tempdir().unwrap();
        let path = run(home.path(), kind, true);
        assert!(!path.exists(), "{kind} created {}", path.display());
    }
}

fn state(kind: &str, payload: Value) -> Option<AgentState> {
    normalize(NormalizeInput {
        kind,
        session: "s",
        parent: "p",
        payload: &payload,
    })
    .unwrap()
    .map(|event| event.state)
}

#[test]
fn gemini_turn_events_map_to_running_waiting_and_done() {
    let event = |name: &str| json!({"hook_event_name": name, "session_id": "g"});
    assert_eq!(
        state("gemini", event("BeforeAgent")),
        Some(AgentState::Running)
    );
    assert_eq!(
        state("gemini", event("AfterTool")),
        Some(AgentState::Running)
    );
    assert_eq!(
        state("gemini", event("AfterAgent")),
        Some(AgentState::Completed)
    );
    assert_eq!(
        state("gemini", event("SessionEnd")),
        Some(AgentState::Stopped)
    );
    let permission = json!({
        "hook_event_name": "Notification",
        "notification_type": "ToolPermission",
        "session_id": "g",
        "message": "Allow shell?"
    });
    assert_eq!(
        state("gemini", permission),
        Some(AgentState::WaitingPermission)
    );
}

#[test]
fn claude_style_agents_map_permission_and_stop() {
    for kind in ["droid", "qwen"] {
        let notification = json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "session_id": "d"
        });
        assert_eq!(
            state(kind, notification),
            Some(AgentState::WaitingPermission)
        );
        let stop = json!({"hook_event_name": "Stop", "session_id": "d"});
        assert_eq!(state(kind, stop), Some(AgentState::Completed));
    }
    let copilot = json!({"hook_event_name": "UserPromptSubmit", "session_id": "c"});
    assert_eq!(state("copilot", copilot), Some(AgentState::Running));
}
