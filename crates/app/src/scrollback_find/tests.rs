use super::*;
use crate::Paths;
use eframe::egui;
use terminator_core::{Lifecycle, Session, SessionKind};

fn query(text: &str, match_case: bool, regex: bool) -> Query {
    Query {
        text: text.into(),
        match_case,
        regex,
    }
}

fn session(id: &str, project: &str, created: u64) -> Session {
    Session {
        review: false,
        remote: None,
        id: id.into(),
        project_id: project.into(),
        label: id.into(),
        cwd: "/".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Ended,
        created,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: String::new(),
        pid: None,
        truncated: false,
        cwd_confirmed: false,
    }
}

#[test]
fn plain_search_follows_the_case_toggle_and_regex_is_optional() {
    let text = "Build OK\nbuild failed\nnothing";
    let lines = |q: Query| -> Vec<usize> {
        let matcher = Matcher::new(&q).unwrap();
        scan_text("s", text, &matcher, 10)
            .0
            .into_iter()
            .map(|hit| hit.line)
            .collect()
    };
    assert_eq!(lines(query("build", false, false)), [0, 1]);
    assert_eq!(lines(query("build", true, false)), [1]);
    assert_eq!(lines(query("^b.*(ok|failed)$", false, true)), [0, 1]);
    assert_eq!(lines(query("^b.*(ok|failed)$", true, true)), [1]);
    assert!(Matcher::new(&query("(", false, true)).is_err());
    assert!(Matcher::new(&query("", false, false)).is_err());
}

#[test]
fn a_scan_is_bounded_in_rows_and_line_length() {
    let long = "x".repeat(LINE_CHARS * 3);
    let text = format!("{long}\n{long}\n{long}\n");
    let matcher = Matcher::new(&query("x", false, false)).unwrap();
    let (hits, more) = scan_text("s", &text, &matcher, 2);
    assert_eq!(hits.len(), 2);
    assert!(more, "the third match is reported as more, not kept");
    assert!(
        hits.iter()
            .all(|hit| hit.text.chars().count() == LINE_CHARS)
    );
}

#[test]
fn the_session_list_is_the_project_or_all_newest_first_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    for n in 0..MAX_SESSIONS + 10 {
        let project = if n % 2 == 0 { "a" } else { "b" };
        let created = u64::try_from(n).unwrap();
        app.state
            .sessions
            .push(session(&format!("s{n}"), project, created));
    }
    let all = app.scrollback_find_sessions(true);
    assert_eq!(all.len(), MAX_SESSIONS);
    assert_eq!(all[0], format!("s{}", MAX_SESSIONS + 9), "newest first");
    let none_selected = app.scrollback_find_sessions(false);
    assert!(none_selected.is_empty(), "no project selected: nothing");
}

#[test]
fn starting_a_search_never_blocks_the_gui_thread() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    for n in 0..50 {
        app.state.sessions.push(session(&format!("s{n}"), "a", n));
    }
    app.scrollback_find.all_projects = true;
    app.scrollback_find.query = query("needle", false, false);
    let started = std::time::Instant::now();
    app.start_scrollback_find(&ctx);
    assert!(
        started.elapsed() < std::time::Duration::from_millis(200),
        "the GUI thread only submits the job"
    );
    assert!(app.scrollback_find.running);
    // No daemon runs here: every read fails, and the job still finishes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app.scrollback_find.running && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.scrollback_find.poll();
    }
    let found = app.scrollback_find.result.clone().unwrap().unwrap();
    assert_eq!(found.hits.len(), 0);
    assert_eq!(found.searched.saturating_add(found.failed), 50);
}

#[test]
fn a_newer_search_replaces_the_older_result() {
    let mut page = FindPage {
        generation: 2,
        running: true,
        ..FindPage::default()
    };
    *page.slot.lock().unwrap() = Some((1, Ok(Found::default())));
    page.poll();
    assert!(
        page.running && page.result.is_none(),
        "stale result dropped"
    );
    *page.slot.lock().unwrap() = Some((2, Ok(Found::default())));
    page.poll();
    assert!(!page.running && page.result.is_some());
}
