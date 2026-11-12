//! Failed Git jobs: lists the panel loads by itself stay in the panel; a
//! user job names its operation in one status line and keeps Retry.
use super::*;
use crate::workspace_ops::Op;

fn repository() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    (app, ctx, dir, requests)
}

fn fail(app: &mut App, ctx: &egui::Context, op: Op, error: &str) {
    let root = app.op_root().expect("an operation root");
    app.update_tx
        .send(Update::Workspace(root, op, Err(error.into())))
        .unwrap();
    drain_updates(app, ctx);
}

#[test]
fn a_new_repository_shows_an_empty_history_and_no_status_error() {
    let (mut app, ctx, _dir, _requests) = repository();
    fail(
        &mut app,
        &ctx,
        Op::Log,
        "Command failed (exit status: 128): fatal: your current branch 'master' does not have any commits yet\n",
    );
    assert_eq!(app.error.text(), None);
    assert_eq!(app.git.list_error, None);
    assert!(app.git.log.is_empty());
}

#[test]
fn a_failed_branch_list_shows_in_the_git_panel_and_the_log_only() {
    let (mut app, ctx, _dir, _requests) = repository();
    fail(
        &mut app,
        &ctx,
        Op::Branches,
        "Command failed (exit status: 128): fatal: bad object\n",
    );
    assert_eq!(app.error.text(), None, "the app started it, not the user");
    assert_eq!(
        app.git.list_error.as_deref(),
        Some("Branches failed: fatal: bad object")
    );
    assert!(app.error.log().any(|entry| entry.error));
}

#[cfg(feature = "test-support")]
fn paint_chip(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| app.status_error_chip(ui));
            });
        },
    );
    output.textures_delta.clear();
}

#[cfg(feature = "test-support")]
fn click(app: &mut App, ctx: &egui::Context, name: &str) {
    let pos = agent_target(ctx, name)
        .unwrap_or_else(|| panic!("{name} is painted"))
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    paint_chip(app, ctx, vec![egui::Event::PointerMoved(pos)]);
    paint_chip(app, ctx, vec![press(true)]);
    paint_chip(app, ctx, vec![press(false)]);
}

#[test]
#[cfg(feature = "test-support")]
fn a_failed_stage_is_one_line_and_retry_queues_the_same_job() {
    let (mut app, ctx, _dir, requests) = repository();
    fail(
        &mut app,
        &ctx,
        Op::Stage("/a/x.rs".into()),
        "Command failed (exit status: 128): fatal: Unable to create index.lock\nAnother git process seems to be running\n",
    );
    assert_eq!(
        app.error.summary().as_deref(),
        Some("Stage failed: fatal: Unable to create index.lock")
    );
    paint_chip(&mut app, &ctx, vec![]);
    paint_chip(&mut app, &ctx, vec![]);
    let chip = agent_target(&ctx, "status-error-chip").unwrap();
    assert!(
        chip.height() <= appearance::TOOLBAR_BUTTON + 0.5,
        "one row: {chip:?}"
    );
    click(&mut app, &ctx, "status-error-chip");
    assert!(app.error.details_open);
    paint_chip(&mut app, &ctx, vec![]);
    while requests.try_recv().is_ok() {}
    click(&mut app, &ctx, "status-error-retry");
    let retried = requests.try_iter().any(|job| {
        matches!(job, Job::Workspace(_, Op::Stage(path)) if path == std::path::Path::new("/a/x.rs"))
    });
    assert!(retried, "Retry queues the same Stage job");
    assert_eq!(app.error.text(), None);
}

#[test]
fn a_create_pr_url_stays_in_the_log_after_the_note_expires() {
    let (mut app, ctx, _dir, _requests) = repository();
    app.info = Some("Created https://example.com/acme/app/pull/42".into());
    app.expire_info(&ctx);
    app.info = None;
    app.expire_info(&ctx);
    assert!(
        app.error
            .log()
            .any(|entry| !entry.error && entry.text.contains("/pull/42"))
    );
}
