use super::*;
#[test]
fn installers_preserve_unrelated_hooks_and_are_idempotent() {
    for kind in ["claude", "grok", "muse"] {
        let home = tempfile::tempdir().unwrap();
        let path = config_path(home.path(), kind).unwrap();
        atomic_write(&path,br#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#).unwrap();
        let helper = Path::new("/tmp/space dir/hook");
        install(home.path(), kind, helper, false).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        install(home.path(), kind, helper, false).unwrap();
        assert_eq!(once, fs::read_to_string(&path).unwrap());
        install(home.path(), kind, helper, true).unwrap();
        let after = fs::read_to_string(path).unwrap();
        assert!(after.contains("echo mine"));
        assert!(!after.contains(MARKER));
    }
}
#[test]
fn backups_within_one_second_do_not_overwrite_each_other() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("settings.json");
    let first = backup_path(&path);
    fs::write(&first, "original").unwrap();
    let second = backup_path(&path);
    assert_ne!(first, second);
    assert_eq!(fs::read_to_string(first).unwrap(), "original");
}
#[cfg(unix)]
#[test]
fn rewriting_a_config_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("settings.json");
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    write_config(&path, b"{\"a\":1}").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
}
#[test]
fn codex_preserves_comments_and_hooks() {
    let source = "# user comment\nmodel = \"example\"\n[[hooks.Stop]]\nhooks = [{type=\"command\", command=\"echo user\"}]\n";
    let once = codex_config(source, &format!("/tmp/hook # {MARKER}"), false).unwrap();
    let twice = codex_config(&once, &format!("/tmp/hook # {MARKER}"), false).unwrap();
    assert_eq!(once, twice);
    let clean = codex_config(&twice, "", true).unwrap();
    assert!(clean.contains("# user comment"));
    assert!(clean.contains("echo user"));
    assert!(!clean.contains(MARKER));
}
#[test]
fn anonymous_and_unrelated_events_not_guessed() {
    assert!(
        normalize("claude", "s", "p", &json!({"hook_event_name":"Stop"}))
            .unwrap()
            .is_none()
    );
    assert!(normalize("claude","s","p",&json!({"hook_event_name":"Notification","notification_type":"auth_success","session_id":"a"})).unwrap().is_none());
}
#[test]
fn permission_and_resume_mapping() {
    let e = normalize(
        "claude",
        "s",
        "p",
        &json!({"hook_event_name":"PermissionRequest","session_id":"a"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(e.state, AgentState::WaitingPermission);
    assert_eq!(e.resume.unwrap().args, vec!["--resume", "a"]);
}
#[test]
fn grok_camel_case_permission_notification() {
    let e = normalize(
        "grok",
        "s",
        "p",
        &json!({
            "hookEventName": "notification",
            "sessionId": "abc",
            "notificationType": "permission_prompt"
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(e.state, AgentState::WaitingPermission);
    assert_eq!(e.provider_session_id.as_deref(), Some("abc"));
}
#[test]
fn codex_installs_notification_hooks() {
    let config = codex_config("", "/tmp/hook # terminator-managed:v1", false).unwrap();
    assert!(config.contains("[[hooks.Notification]]"));
    assert!(config.contains("[[hooks.PermissionRequest]]"));
}
#[test]
fn opencode_plugin_forwards_permission_and_question_events() {
    let home = tempfile::tempdir().unwrap();
    let helper = Path::new("/tmp/space dir/hook");
    let path = install(home.path(), "opencode", helper, false).unwrap();
    let once = fs::read_to_string(&path).unwrap();
    assert!(once.contains(MARKER));
    assert!(once.contains("permission.v2.asked"));
    assert!(once.contains("question.v2.asked"));
    assert!(once.contains("question.rejected"));
    assert!(once.contains("export default"));
    assert!(once.contains("async setup(ctx)"));
    assert!(once.contains("async function server"));
    assert!(once.contains("/tmp/space dir/hook"));
    install(home.path(), "opencode", helper, false).unwrap();
    assert_eq!(once, fs::read_to_string(&path).unwrap());
    install(home.path(), "opencode", helper, true).unwrap();
    assert!(!path.exists());
}
#[test]
fn opencode_permission_and_question_map_like_notifications() {
    let perm = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "permission.asked",
            "properties": { "id": "req-1", "sessionID": "oc-1" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(perm.state, AgentState::WaitingPermission);
    assert_eq!(perm.provider_session_id.as_deref(), Some("oc-1"));
    assert_eq!(perm.request_id.as_deref(), Some("req-1"));
    assert_eq!(perm.resume.unwrap().args, vec!["--session", "oc-1"]);

    let perm_v2 = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "permission.v2.asked",
            "data": { "id": "req-2", "sessionID": "oc-2" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(perm_v2.state, AgentState::WaitingPermission);
    assert_eq!(perm_v2.provider_session_id.as_deref(), Some("oc-2"));
    assert_eq!(perm_v2.request_id.as_deref(), Some("req-2"));

    let question = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "question.asked",
            "properties": { "id": "q-1", "sessionID": "oc-1" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(question.state, AgentState::WaitingInput);

    let question_v2 = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "question.v2.asked",
            "data": { "id": "q-2", "sessionID": "oc-2" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(question_v2.state, AgentState::WaitingInput);

    let replied = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "question.v2.replied",
            "data": { "requestID": "q-2", "sessionID": "oc-2" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(replied.state, AgentState::Running);

    let rejected = normalize(
        "opencode",
        "s",
        "p",
        &json!({
            "type": "question.rejected",
            "properties": { "requestID": "q-1", "sessionID": "oc-1" }
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(rejected.state, AgentState::Running);
}
#[test]
fn muse_settings_keep_user_keys_and_subagent_hooks() {
    let home = tempfile::tempdir().unwrap();
    let settings = home.path().join(".config/muse/settings.json");
    atomic_write(
        &settings,
        br#"{"schema_version":1,"model":"muse-spark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
    )
    .unwrap();
    let helper = Path::new("/tmp/space dir/hook");
    install(home.path(), "muse", helper, false).unwrap();
    let once = fs::read_to_string(&settings).unwrap();
    assert!(once.contains(MARKER));
    assert!(once.contains("SubagentStop"));
    assert!(once.contains("echo mine"));
    assert!(once.contains("muse-spark"));
    assert!(
        fs::read_to_string(config_path(home.path(), "muse").unwrap())
            .unwrap()
            .contains("SubagentStop")
    );
    install(home.path(), "muse", helper, false).unwrap();
    assert_eq!(once, fs::read_to_string(&settings).unwrap());
    assert!(installed(home.path(), "muse"));
    install(home.path(), "muse", helper, true).unwrap();
    let after = fs::read_to_string(&settings).unwrap();
    assert!(after.contains("echo mine"));
    assert!(after.contains("muse-spark"));
    assert!(!after.contains(MARKER));
    assert!(!installed(home.path(), "muse"));
}
#[test]
fn muse_subagent_stop_completes() {
    let event = normalize(
        "muse",
        "s",
        "p",
        &json!({"hook_event_name":"SubagentStop","session_id":"child"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(event.state, AgentState::Completed);
    assert_eq!(event.provider_session_id.as_deref(), Some("child"));
}
#[test]
fn opencode_unknown_events_ignored() {
    assert!(
        normalize(
            "opencode",
            "s",
            "p",
            &json!({"type":"file.edited","properties":{"sessionID":"x"}})
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn cursor_hooks_keep_user_commands_and_skip_permission_events() {
    let home = tempfile::tempdir().unwrap();
    let path = config_path(home.path(), "cursor").unwrap();
    atomic_write(
        &path,
        br#"{"version":1,"extra":true,"hooks":{"stop":[{"command":"echo mine"}],"beforeShellExecution":[{"command":"echo gate"}]}}"#,
    )
    .unwrap();
    let helper = Path::new("/tmp/space dir/hook");
    install(home.path(), "cursor", helper, false).unwrap();
    let once = fs::read_to_string(&path).unwrap();
    install(home.path(), "cursor", helper, false).unwrap();
    assert_eq!(once, fs::read_to_string(&path).unwrap());
    let doc: Value = serde_json::from_str(&once).unwrap();
    assert_eq!(doc["version"].as_u64(), Some(1));
    assert_eq!(doc["extra"].as_bool(), Some(true));
    let hooks = doc["hooks"].as_object().unwrap();
    for event in [
        "sessionStart",
        "beforeSubmitPrompt",
        "postToolUse",
        "postToolUseFailure",
        "afterAgentResponse",
        "stop",
        "sessionEnd",
    ] {
        assert!(
            hooks[event]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["command"].as_str().unwrap_or("").contains(MARKER)),
            "{event}"
        );
    }
    assert!(
        hooks["stop"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["command"] == "echo mine")
    );
    assert_eq!(hooks["beforeShellExecution"][0]["command"], "echo gate");
    assert!(!hooks.contains_key("preToolUse"));
    assert!(!hooks.contains_key("beforeReadFile"));
    assert!(!hooks.contains_key("subagentStart"));
    let prompt = hooks["beforeSubmitPrompt"][0]["command"].as_str().unwrap();
    assert!(prompt.contains("{\"continue\":true}"));
    assert!(prompt.contains("/tmp/space dir/hook"));
    assert_eq!(hooks["sessionStart"][0]["timeout"].as_u64(), Some(2));
    let observe = hooks["stop"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["command"].as_str().unwrap_or("").contains(MARKER))
        .unwrap()["command"]
        .as_str()
        .unwrap();
    assert!(observe.contains("printf '%s\\n' '{}'"));
    install(home.path(), "cursor", helper, true).unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("echo mine"));
    assert!(after.contains("echo gate"));
    assert!(after.contains("\"extra\": true"));
    assert!(!after.contains(MARKER));
    assert!(!installed(home.path(), "cursor"));
}
#[test]
fn cursor_rejects_unknown_schema_and_nested_groups() {
    let home = tempfile::tempdir().unwrap();
    let path = config_path(home.path(), "cursor").unwrap();
    let versioned = "{\"version\":2,\"hooks\":{}}\n";
    atomic_write(&path, versioned.as_bytes()).unwrap();
    let helper = Path::new("/tmp/hook");
    assert!(install(home.path(), "cursor", helper, false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), versioned);
    atomic_write(
        &path,
        br#"{"version":1,"hooks":{"stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
    )
    .unwrap();
    let before = fs::read_to_string(&path).unwrap();
    assert!(install(home.path(), "cursor", helper, false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}
#[test]
fn cursor_cli_session_end_reports_the_real_outcome_without_a_stop_hook() {
    // Sanitized payload from the real Cursor CLI Reply OK check.
    let payload: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/cursor-session-end-completed.json"
    ))
    .unwrap();
    let completed = normalize("cursor", "s", "p", &payload).unwrap().unwrap();
    assert_eq!(completed.state, AgentState::Completed);
    assert_eq!(completed.summary, "Completed");
    assert!(completed.state.actionable());
    for (reason, expected) in [
        ("error", AgentState::Failed),
        ("aborted", AgentState::Unknown),
        ("window_close", AgentState::Stopped),
        ("user_close", AgentState::Stopped),
        ("unrecognized", AgentState::Stopped),
    ] {
        let mut ended = payload.clone();
        ended["reason"] = json!(reason);
        // A stale final_status must not turn a user-close into completion.
        assert_eq!(
            normalize("cursor", "s", "p", &ended)
                .unwrap()
                .unwrap()
                .state,
            expected
        );
    }
    assert_eq!(
        normalize("claude", "s", "p", &payload)
            .unwrap()
            .unwrap()
            .state,
        AgentState::Stopped
    );
}

#[test]
fn cursor_events_map_conversation_without_prompt_text() {
    let completed = normalize(
        "cursor",
        "s",
        "p",
        &json!({
            "hook_event_name": "stop",
            "conversation_id": "chat-1",
            "status": "completed"
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(completed.state, AgentState::Completed);
    assert_eq!(completed.provider_session_id.as_deref(), Some("chat-1"));
    let resume = completed.resume.unwrap();
    assert_eq!(resume.program, "cursor-agent");
    assert_eq!(resume.args, vec!["--resume", "chat-1"]);
    let failed = normalize(
        "cursor",
        "s",
        "p",
        &json!({"hook_event_name":"stop","conversation_id":"chat-1","status":"error"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(failed.state, AgentState::Failed);
    let aborted = normalize(
        "cursor",
        "s",
        "p",
        &json!({"hook_event_name":"stop","conversation_id":"chat-1","status":"aborted"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(aborted.state, AgentState::Unknown);
    let prompt = normalize(
        "cursor",
        "s",
        "p",
        &json!({
            "hook_event_name": "beforeSubmitPrompt",
            "conversation_id": "chat-1",
            "prompt": "secret token"
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(prompt.state, AgentState::Running);
    assert_eq!(prompt.summary, "Working");
    assert!(!prompt.details.contains("secret"));
    let ended = normalize(
        "cursor",
        "s",
        "p",
        &json!({"hook_event_name":"sessionEnd","session_id":"chat-1","reason":"user_close"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(ended.state, AgentState::Stopped);
    assert!(
        normalize(
            "cursor",
            "s",
            "p",
            &json!({"hook_event_name":"workspaceOpen","cursor_version":"1.0"})
        )
        .unwrap()
        .is_none()
    );
}
