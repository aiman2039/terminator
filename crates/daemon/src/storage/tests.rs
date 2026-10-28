use super::*;
#[test]
fn rotation_retention_clear_and_remove_ordering() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    let sid = id();
    let mut history = History::new(paths.clone()).unwrap();
    history.append(&sid, &vec![b'a'; 4 * 1024 * 1024]).unwrap();
    history.append(&sid, b"tail").unwrap();
    history.flush().unwrap();
    let files = history_files(&paths, Some(&sid));
    assert_eq!(files.len(), 2);
    assert_eq!(fs::read(&files[1]).unwrap(), b"tail");
    assert_eq!(
        history
            .prune(&Settings {
                session_mib: 1,
                ..Default::default()
            })
            .unwrap(),
        vec![sid.clone()]
    );
    assert_eq!(history_files(&paths, Some(&sid)).len(), 1);
    history.clear(Some(&sid), false).unwrap();
    history.append(&sid, b"after clear").unwrap();
    history.flush().unwrap();
    assert_eq!(
        fs::read(&history_files(&paths, Some(&sid))[0]).unwrap(),
        b"after clear"
    );
    history.clear(Some(&sid), true).unwrap();
    history.append(&sid, b"late producer").unwrap();
    history.flush().unwrap();
    assert!(history_files(&paths, Some(&sid)).is_empty());
}
#[test]
fn age_rotation_and_existing_segments() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    let sid = id();
    let legacy = paths.history_dir().join(format!("{sid}-{:020}.pty", now()));
    fs::write(&legacy, b"legacy").unwrap();
    let mut history = History::new(paths.clone()).unwrap();
    history.append(&sid, b"new").unwrap();
    history.writers.get_mut(&sid).unwrap().opened -= std::time::Duration::from_secs(301);
    history.append(&sid, b"rotated").unwrap();
    history.flush().unwrap();
    assert_eq!(history_files(&paths, Some(&sid)).len(), 3);
    history.segments.get_mut(&legacy).unwrap().created = 0;
    history.prune(&Settings::default()).unwrap();
    assert!(!legacy.exists());
}
#[test]
fn sqlite_roundtrip_and_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let (db, mut s) = Store::open(&p).unwrap();
    s.projects.push(Project {
        id: id(),
        name: "test".into(),
        path: "/tmp".into(),
        layout: serde_json::Value::Null,
    });
    db.save(&s).unwrap();
    drop(db);
    let (_, t) = Store::open(&p).unwrap();
    assert_eq!(t.projects.len(), 1);
}
#[test]
fn presence_observations_are_never_persisted() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let (db, mut s) = Store::open(&p).unwrap();
    s.presence.push(agents::TerminalPresence {
        session_id: "live".into(),
        generation: s.generation.clone(),
        agents: vec![agents::DetectedAgent {
            kind: "codex".into(),
            process: agents::ProcessIdentity {
                pid: 1,
                start_time: 1,
            },
            foreground: true,
        }],
        outcome: agents::PresenceOutcome::Verified,
        observed_at: now(),
    });
    db.save(&s).unwrap();
    drop(db);
    let (_, t) = Store::open(&p).unwrap();
    assert!(t.presence.is_empty());
    let conn = Connection::open(tmp.path().join("state.sqlite3")).unwrap();
    let json: String = conn
        .query_row("SELECT json FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(stored["presence"], serde_json::json!([]));
}
#[test]
fn history_replay_has_no_escape_side_effects() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let sid = id();
    let mut history = History::new(p.clone()).unwrap();
    history
        .append(&sid, b"safe\x1b]52;c;aGVsbG8=\x07\r\ntext")
        .unwrap();
    history.flush().unwrap();
    let s = Session {
        remote: None,
        review: false,
        id: sid,
        project_id: id(),
        label: String::new(),
        cwd: "/tmp".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Ended,
        created: now(),
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: id(),
        pid: None,
        truncated: false,
        cwd_confirmed: false,
    };
    let t = text(&p, &s).unwrap();
    assert!(t.contains("safe"));
    assert!(!t.contains('\x1b'));
}
fn corrupt_names(data: &std::path::Path) -> Vec<String> {
    fs::read_dir(data)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("state.corrupt-") && n.ends_with(".sqlite3"))
        .collect()
}
#[test]
fn unreadable_store_is_quarantined_and_starts_with_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    fs::write(p.data.join("state.sqlite3"), b"not a database at all").unwrap();
    let (store, state) = Store::open(&p).unwrap();
    assert_eq!(state.settings, Settings::default());
    assert!(
        state.degraded.as_deref().unwrap().contains("quarantined"),
        "startup must report the quarantine: {:?}",
        state.degraded
    );
    assert_eq!(corrupt_names(&p.data).len(), 1);
    // The fresh store is fully usable: saves land and reload.
    let mut fresh = State::default();
    fresh.settings.font_size = 16.0;
    store.save(&fresh).unwrap();
    drop(store);
    let (_, reloaded) = Store::open(&p).unwrap();
    assert_eq!(reloaded.settings.font_size, 16.0);
    assert!(reloaded.degraded.is_none());
}
#[test]
fn invalid_json_row_is_quarantined_not_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let (db, _) = Store::open(&p).unwrap();
    db.conn
        .execute("INSERT INTO app_state(id,json) VALUES(1,'{truncated')", [])
        .unwrap();
    drop(db);
    let (_, state) = Store::open(&p).unwrap();
    assert_eq!(state.settings, Settings::default());
    assert!(
        state.degraded.as_deref().unwrap().contains("quarantined"),
        "a half-written row must not fail startup: {:?}",
        state.degraded
    );
    assert_eq!(corrupt_names(&p.data).len(), 1);
}
#[test]
fn invalid_settings_are_reset_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let (db, _) = Store::open(&p).unwrap();
    let mut damaged = State::default();
    damaged.settings.history_days = 0;
    assert!(damaged.settings.validate().is_err());
    db.save(&damaged).unwrap();
    drop(db);
    let (_, state) = Store::open(&p).unwrap();
    assert_eq!(state.settings, Settings::default());
    assert!(
        state.degraded.as_deref().unwrap().contains("validation"),
        "settings reset must be reported: {:?}",
        state.degraded
    );
    assert!(corrupt_names(&p.data).is_empty());
}
#[test]
fn newer_database_still_fails_without_quarantine() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().into());
    p.init().unwrap();
    let (db, _) = Store::open(&p).unwrap();
    db.conn.execute_batch("PRAGMA user_version=99").unwrap();
    drop(db);
    match Store::open(&p) {
        Err(error) => assert!(
            format!("{error:#}").contains("newer than this application"),
            "unexpected error: {error:#}"
        ),
        Ok(_) => panic!("a newer database must fail instead of opening"),
    }
    assert!(
        corrupt_names(&p.data).is_empty(),
        "a newer database must never be clobbered"
    );
}
