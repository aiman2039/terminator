//! Commit Log filters in the app: the first load, filter edits, the typing
//! pause, stale and failed replies, Go to Hash, and SSH projects.
use super::*;
use crate::git_log::{CommitEntry, CommitLog, GraphRow, LogDate, LogQuery};

fn log_tab() -> Tab {
    Tab::CommitLog { cwd: "/a".into() }
}

/// One frame of the Commit Log tab; returns the painted text.
fn paint(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 700.0),
        )),
        events,
        ..Default::default()
    };
    let tab = log_tab();
    let mut output = ctx.run_ui(input, |ui| app.commit_log_view(ui, &tab));
    output.textures_delta.clear();
    painted_strings(&output.shapes)
}

/// Log queries sent since the last call. The Commit Log never sends a
/// daemon request, so it works the same with an old daemon.
fn sent(requests: &mpsc::Receiver<Job>) -> Vec<LogQuery> {
    let mut queries = Vec::new();
    for job in requests.try_iter() {
        match job {
            Job::GitLog(_, query) => queries.push(query),
            Job::Control(..) => panic!("the Commit Log sent a daemon request"),
            _ => {}
        }
    }
    queries
}

fn commit(hash: &str, subject: &str) -> CommitEntry {
    CommitEntry {
        hash: hash.repeat(40),
        short_hash: hash.repeat(7),
        author: "Ada".into(),
        email: String::new(),
        date: String::new(),
        timestamp: 0,
        message: subject.into(),
        subject: subject.into(),
        refs: Vec::new(),
        parents: Vec::new(),
        graph: GraphRow::default(),
    }
}

fn answer(app: &mut App, ctx: &egui::Context, log: CommitLog) {
    let key = log_tab().key();
    app.update_tx
        .send(Update::GitView(key, GitView::Log(log)))
        .unwrap();
    drain_updates(app, ctx);
}

fn reply(query: &LogQuery, commits: Vec<CommitEntry>) -> CommitLog {
    CommitLog {
        commits,
        query: Box::new(query.clone()),
        ..CommitLog::default()
    }
}

/// The app after the first load answered with commits `a` and `b`.
fn loaded() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    // An old daemon advertises nothing; the log needs nothing from it.
    app.state.capabilities.clear();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    paint(&mut app, &ctx, vec![]);
    let first = sent(&requests);
    assert_eq!(first.len(), 1, "one first load");
    assert!(first[0].filter.is_empty());
    assert!(matches!(
        app.git_views.get(&log_tab().key()),
        Some(GitView::Loading)
    ));
    let commits = vec![commit("a", "fix one"), commit("b", "docs")];
    answer(&mut app, &ctx, reply(&first[0], commits));
    (app, ctx, dir, requests)
}

fn shown_subjects(app: &App) -> Vec<String> {
    match app.git_views.get(&log_tab().key()) {
        Some(GitView::Log(log)) => log.commits.iter().map(|c| c.subject.clone()).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_filter_pick_loads_at_once_and_the_shown_log_stays_meanwhile() {
    let (mut app, ctx, _dir, requests) = loaded();
    assert_eq!(shown_subjects(&app), ["fix one", "docs"]);
    let view = app.git.logs.get_mut(&log_tab().key()).unwrap();
    view.draft.date = LogDate::Week;
    view.apply();
    paint(&mut app, &ctx, vec![]);
    let queries = sent(&requests);
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].filter.date, LogDate::Week);
    assert_eq!(shown_subjects(&app), ["fix one", "docs"]);
    assert!(app.git.logs[&log_tab().key()].loading);
    answer(
        &mut app,
        &ctx,
        reply(&queries[0], vec![commit("b", "docs")]),
    );
    assert_eq!(shown_subjects(&app), ["docs"]);
    let texts = paint(&mut app, &ctx, vec![]);
    assert!(
        texts.iter().any(|t| t == "1 commit matches the filters."),
        "{texts:?}"
    );
    assert!(sent(&requests).is_empty(), "nothing changed: no new load");
}

#[test]
fn stale_replies_are_dropped_and_a_failed_filter_keeps_the_shown_log() {
    let (mut app, ctx, _dir, requests) = loaded();
    let key = log_tab().key();
    let view = app.git.logs.get_mut(&key).unwrap();
    view.draft.author = Some("Ada".into());
    view.apply();
    paint(&mut app, &ctx, vec![]);
    let older = sent(&requests).remove(0);
    let view = app.git.logs.get_mut(&key).unwrap();
    view.draft.text = "fix".into();
    view.apply();
    paint(&mut app, &ctx, vec![]);
    let newer = sent(&requests).remove(0);
    answer(&mut app, &ctx, reply(&older, vec![commit("c", "stale")]));
    assert_eq!(shown_subjects(&app), ["fix one", "docs"]);
    answer(&mut app, &ctx, newer.failed("git log: bad pattern".into()));
    assert_eq!(shown_subjects(&app), ["fix one", "docs"]);
    let texts = paint(&mut app, &ctx, vec![]);
    assert!(
        texts.iter().any(|t| t == "git log: bad pattern"),
        "{texts:?}"
    );
    assert!(!app.git.logs[&key].loading);
}

#[test]
fn a_first_load_that_ends_without_a_reply_shows_why_once() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    paint(&mut app, &ctx, vec![]);
    let first = sent(&requests).remove(0);
    let key = log_tab().key();
    // The end of a replaced query changes nothing.
    app.log_job_ended(&key, first.generation.saturating_add(9), None);
    assert!(matches!(app.git_views.get(&key), Some(GitView::Loading)));
    let error = "Log query stopped: Operation timed out";
    app.log_job_ended(&key, first.generation, Some(error.into()));
    assert!(!app.git.logs[&key].loading);
    let texts = paint(&mut app, &ctx, vec![]);
    let shown = texts.iter().filter(|text| *text == error).count();
    assert_eq!(shown, 1, "{texts:?}");
    assert!(
        sent(&requests).is_empty(),
        "Reload loads again, not each frame"
    );
}

#[test]
fn a_filtered_load_that_ends_without_a_reply_says_so_over_the_shown_log() {
    let (mut app, ctx, _dir, requests) = loaded();
    let key = log_tab().key();
    let view = app.git.logs.get_mut(&key).unwrap();
    view.draft.date = LogDate::Week;
    view.apply();
    paint(&mut app, &ctx, vec![]);
    let query = sent(&requests).remove(0);
    // The end of a replaced query changes nothing.
    app.log_job_ended(&key, query.generation.saturating_add(1), None);
    assert!(app.git.logs[&key].loading);
    // Quitting stops reads: the query the bar shows ends without a reply.
    app.log_job_ended(&key, query.generation, None);
    assert!(!app.git.logs[&key].loading);
    assert_eq!(shown_subjects(&app), ["fix one", "docs"]);
    let texts = paint(&mut app, &ctx, vec![]);
    let note = "Loading stopped. Click Reload.";
    assert_eq!(texts.iter().filter(|text| *text == note).count(), 1);
    assert!(sent(&requests).is_empty(), "Reload loads again");
}

#[test]
fn ssh_projects_get_a_note_instead_of_a_commit_log() {
    let (mut app, _ctx, _dir) = fixture();
    let mut remote = crate::remote_ui::blank_project();
    remote.id = "r1".into();
    remote.name = "Build box".into();
    remote.root = "/srv".into();
    app.state.remote_projects.push(remote);
    app.selected = Some("r1".into());
    app.context = Some(services::ContextData {
        cwd: "/srv".into(),
        root: Some("/srv".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    assert!(!app.git_action_applies(Action::GitShowLog));
    app.open_commit_log();
    assert_eq!(app.info.as_deref(), Some(crate::git_log::REMOTE_LOG_NOTE));
    let log = Tab::CommitLog { cwd: "/srv".into() };
    assert!(!app.layouts.get("r1").is_some_and(|w| w.contains(&log)));
}

#[cfg(feature = "test-support")]
fn click(app: &mut App, ctx: &egui::Context, name: &str) {
    let at = crate::diagnostics::current_rect(ctx, name)
        .unwrap()
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    paint(app, ctx, vec![egui::Event::PointerMoved(at)]);
    paint(app, ctx, vec![press(true)]);
    paint(app, ctx, vec![press(false)]);
}

#[cfg(feature = "test-support")]
#[test]
fn typing_a_filter_waits_for_a_pause_and_enter_loads_at_once() {
    let (mut app, ctx, _dir, requests) = loaded();
    click(&mut app, &ctx, "log-text");
    paint(&mut app, &ctx, vec![egui::Event::Text("fix".into())]);
    assert_eq!(app.git.logs[&log_tab().key()].draft.text, "fix");
    assert!(sent(&requests).is_empty(), "typing waits for a pause");
    std::thread::sleep(
        crate::git_log::TYPING_PAUSE
            .checked_add(Duration::from_millis(100))
            .unwrap(),
    );
    paint(&mut app, &ctx, vec![]);
    let queries = sent(&requests);
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].filter.text, "fix");
    paint(&mut app, &ctx, vec![egui::Event::Text(" one".into())]);
    assert!(sent(&requests).is_empty());
    paint(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    let queries = sent(&requests);
    assert_eq!(queries.len(), 1, "Enter loads at once");
    assert_eq!(queries[0].filter.text, "fix one");
}

#[cfg(feature = "test-support")]
#[test]
fn go_to_hash_selects_a_loaded_commit_or_loads_it() {
    let (mut app, ctx, _dir, requests) = loaded();
    click(&mut app, &ctx, "log-goto");
    paint(&mut app, &ctx, vec![egui::Event::Text("bbbbbbb".into())]);
    paint(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    assert_eq!(app.selected_commit, Some("b".repeat(40)));
    assert!(sent(&requests).is_empty(), "a loaded commit needs no Git");
    let key = log_tab().key();
    app.git.logs.get_mut(&key).unwrap().goto.clear();
    click(&mut app, &ctx, "log-goto");
    paint(&mut app, &ctx, vec![egui::Event::Text("ccccccc".into())]);
    paint(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    let queries = sent(&requests);
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].goto.as_deref(), Some("ccccccc"));
    let found = CommitLog {
        found: Some("c".repeat(40)),
        ..reply(
            &queries[0],
            vec![
                commit("a", "fix one"),
                commit("b", "docs"),
                commit("c", "old"),
            ],
        )
    };
    answer(&mut app, &ctx, found);
    assert_eq!(app.selected_commit, Some("c".repeat(40)));
    assert_eq!(shown_subjects(&app), ["fix one", "docs", "old"]);
}

#[cfg(feature = "test-support")]
#[test]
fn a_newer_go_to_hash_of_a_loaded_commit_keeps_its_selection() {
    let (mut app, ctx, _dir, requests) = loaded();
    let key = log_tab().key();
    click(&mut app, &ctx, "log-goto");
    paint(&mut app, &ctx, vec![egui::Event::Text("ccccccc".into())]);
    paint(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    let older = sent(&requests).remove(0);
    app.git.logs.get_mut(&key).unwrap().goto.clear();
    click(&mut app, &ctx, "log-goto");
    paint(&mut app, &ctx, vec![egui::Event::Text("bbbbbbb".into())]);
    paint(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    assert_eq!(app.selected_commit, Some("b".repeat(40)));
    let commits = vec![
        commit("a", "fix one"),
        commit("b", "docs"),
        commit("c", "old"),
    ];
    let found = CommitLog {
        found: Some("c".repeat(40)),
        ..reply(&older, commits)
    };
    answer(&mut app, &ctx, found);
    assert_eq!(shown_subjects(&app), ["fix one", "docs", "old"]);
    assert_eq!(
        app.selected_commit,
        Some("b".repeat(40)),
        "the older Go to Hash does not move the selection"
    );
}
