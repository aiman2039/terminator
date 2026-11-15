//! Codex quota from session log text and files in a temporary folder.
use super::*;

const WINDOWS: &str = r#"{"timestamp":"2026-11-10T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":42.5,"window_minutes":300,"resets_at":1794319200},"secondary":{"used_percent":7.0,"window_minutes":10080,"resets_in_seconds":3600},"plan_type":"pro"}}}"#;
const NO_WINDOWS: &str = r#"{"timestamp":"2026-11-10T09:05:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":null,"secondary":null,"plan_type":"business"}}}"#;

#[test]
fn the_newest_record_with_windows_gives_used_percent_and_resets() {
    let text = format!("{WINDOWS}\n{NO_WINDOWS}\nnot json\n");
    let quota = codex_quota_from_log(&text).expect("quota");
    assert_eq!(quota.plan.as_deref(), Some("pro"));
    assert_eq!(quota.observed_at, 1_794_301_200);
    assert_eq!(
        quota.windows,
        [
            QuotaWindow {
                label: "5h".into(),
                used_percent: 42.5,
                resets_at: Some(1_794_319_200),
            },
            QuotaWindow {
                label: "weekly".into(),
                used_percent: 7.0,
                resets_at: Some(1_794_301_200 + 3600),
            },
        ]
    );
}

#[test]
fn the_claude_record_gives_both_windows_and_an_empty_one_gives_none() {
    let record = br#"{"five_hour":{"used_percentage":23.5,"resets_at":100},"seven_day":{"used_percentage":41.2,"resets_at":200},"observed_at":7}"#;
    let quota = claude_quota_from_record(record).expect("quota");
    assert_eq!(quota.agent, "claude");
    assert_eq!(quota.observed_at, 7);
    let labels = quota
        .windows
        .iter()
        .map(|w| (w.label.as_str(), w.used_percent, w.resets_at))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [("5h", 23.5, Some(100)), ("weekly", 41.2, Some(200))]
    );
    assert_eq!(claude_quota_from_record(br#"{"observed_at":7}"#), None);
}

#[test]
fn a_plan_without_windows_reports_nothing() {
    assert_eq!(codex_quota_from_log(NO_WINDOWS), None);
    assert_eq!(codex_quota_from_log(""), None);
}

#[test]
fn the_newest_log_file_is_read_first() {
    let dir = tempfile::tempdir().unwrap();
    let day = dir.path().join("2026/11/10");
    std::fs::create_dir_all(&day).unwrap();
    let older = dir.path().join("2026/11/09");
    std::fs::create_dir_all(&older).unwrap();
    std::fs::write(
        older.join("rollout-2026-11-09T23-00-00-a.jsonl"),
        WINDOWS.replace("42.5", "5.0"),
    )
    .unwrap();
    std::fs::write(
        day.join("rollout-2026-11-10T08-00-00-b.jsonl"),
        WINDOWS.replace("42.5", "10.0"),
    )
    .unwrap();
    std::fs::write(day.join("rollout-2026-11-10T09-00-00-c.jsonl"), WINDOWS).unwrap();
    std::fs::write(day.join("notes.txt"), WINDOWS.replace("42.5", "99.0")).unwrap();
    let quota = codex_quota(dir.path()).expect("quota");
    assert_eq!(quota.windows[0].used_percent, 42.5);
    assert_eq!(codex_quota(&dir.path().join("missing")), None);
}
