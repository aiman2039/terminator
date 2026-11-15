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
