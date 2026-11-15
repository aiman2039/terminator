//! Claude Code status-line source against isolated fixture homes.
use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    (dir, home, data)
}

fn settings_file(home: &Path) -> PathBuf {
    settings_path(home).unwrap()
}

fn settings(home: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(settings_file(home)).unwrap()).unwrap()
}

#[test]
fn install_keeps_the_users_status_line_and_remove_restores_it() {
    let (_dir, home, data) = fixture();
    let mine = json!({"type": "command", "command": "~/bin/line.sh", "padding": 1});
    std::fs::write(
        settings_file(&home),
        serde_json::to_string(&json!({"model": "opus", "statusLine": mine})).unwrap(),
    )
    .unwrap();
    install(&home, Path::new("/opt/terminator-hook"), &data, false).unwrap();
    let doc = settings(&home);
    let command = doc["statusLine"]["command"].as_str().unwrap();
    assert!(command.contains("statusline --data-dir") && command.contains(STATUSLINE_MARKER));
    assert_eq!(doc["statusLine"]["padding"], 1);
    assert_eq!(doc["model"], "opus");
    assert!(installed(&home));
    assert_eq!(original_command(&data).as_deref(), Some("~/bin/line.sh"));
    // A second install keeps the saved original.
    install(&home, Path::new("/opt/terminator-hook"), &data, false).unwrap();
    assert_eq!(original_command(&data).as_deref(), Some("~/bin/line.sh"));
    install(&home, Path::new("/opt/terminator-hook"), &data, true).unwrap();
    assert_eq!(settings(&home)["statusLine"], mine);
    assert!(!installed(&home));
    let backups = std::fs::read_dir(home.join(".claude"))
        .unwrap()
        .filter(|e| {
            e.as_ref().is_ok_and(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("terminator-backup")
            })
        })
        .count();
    assert!(backups >= 2);
}

#[test]
fn the_status_line_alone_does_not_count_as_claude_hooks() {
    let (_dir, home, data) = fixture();
    install(&home, Path::new("/h"), &data, false).unwrap();
    assert!(installed(&home));
    assert!(!crate::installed(&home, "claude"));
}

#[test]
fn remove_without_an_original_drops_the_managed_line_only() {
    let (_dir, home, data) = fixture();
    install(&home, Path::new("/h"), &data, false).unwrap();
    assert_eq!(original_command(&data), None);
    install(&home, Path::new("/h"), &data, true).unwrap();
    assert!(settings(&home).get("statusLine").is_none());
}

#[test]
fn a_users_line_is_never_removed_by_remove() {
    let (_dir, home, data) = fixture();
    let mine = json!({"statusLine": {"type": "command", "command": "mine"}});
    std::fs::write(settings_file(&home), mine.to_string()).unwrap();
    install(&home, Path::new("/h"), &data, true).unwrap();
    assert_eq!(settings(&home), mine);
}

#[test]
fn the_status_line_model_becomes_usage_only() {
    let input = json!({
        "session_id": "s",
        "cwd": "/secret",
        "model": {"id": "claude-opus-4-1", "display_name": "Opus"},
        "cost": {"total_cost_usd": 3}
    });
    let found = usage(&input).unwrap();
    assert_eq!(found.model.as_deref(), Some("claude-opus-4-1"));
    assert_eq!(found.permission_mode, None);
    let named = usage(&json!({"model": {"display_name": "Opus"}})).unwrap();
    assert_eq!(named.model.as_deref(), Some("Opus"));
    assert!(usage(&json!({"model": {}})).is_none());
    assert!(usage(&json!({"cwd": "/x"})).is_none());
}

#[test]
fn a_model_is_sent_again_only_after_a_change_or_a_failed_send() {
    let (_dir, _home, data) = fixture();
    let mut sent = SentModels::load(&data);
    assert!(sent.due("t:s", "a", 100));
    sent.record(
        "t:s",
        "a",
        Attempt {
            delivered: true,
            at: 100,
        },
    );
    sent.save(&data).unwrap();
    let sent = SentModels::load(&data);
    assert!(!sent.due("t:s", "a", 10_000), "delivered: never again");
    assert!(sent.due("t:s", "b", 101), "a new model is sent");
    assert!(sent.due("t:other", "a", 101), "another session is its own");
    let mut sent = sent;
    sent.record(
        "t:s",
        "b",
        Attempt {
            delivered: false,
            at: 200,
        },
    );
    assert!(!sent.due("t:s", "b", 200 + RETRY_SECS - 1));
    assert!(sent.due("t:s", "b", 200 + RETRY_SECS));
}

#[test]
fn sent_models_keep_only_the_newest_sessions() {
    let (_dir, _home, data) = fixture();
    let mut sent = SentModels::load(&data);
    for at in 0..(MAX_SENT as u64 + 5) {
        sent.record(
            &format!("t:{at}"),
            "m",
            Attempt {
                delivered: true,
                at,
            },
        );
    }
    assert_eq!(sent.0.len(), MAX_SENT);
    assert!(sent.due("t:0", "m", 1_000), "the oldest was dropped");
    assert!(!sent.due(&format!("t:{}", MAX_SENT as u64 + 4), "m", 1_000));
}

#[test]
fn the_record_keeps_only_the_rate_limit_windows() {
    let input = br#"{"session_id":"s","cwd":"/secret","cost":{"total_cost_usd":3},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":100},"seven_day":{"used_percentage":41.2,"resets_at":200}}}"#;
    let record: Value = serde_json::from_slice(&quota_record(input, 7).unwrap()).unwrap();
    assert_eq!(
        record,
        json!({
            "five_hour": {"used_percentage": 23.5, "resets_at": 100},
            "seven_day": {"used_percentage": 41.2, "resets_at": 200},
            "observed_at": 7
        })
    );
    assert_eq!(quota_record(br#"{"cwd":"/x"}"#, 7), None);
}
