use eframe::egui::{self};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};
use terminator_core::*;

use super::super::*;
#[cfg(feature = "test-support")]
pub(super) fn header_target(ctx: &egui::Context) -> impl Fn(&str) -> Option<egui::Rect> + '_ {
    |name| ctx.data(|data| data.get_temp(egui::Id::new(("fixture-target", name))))
}

#[cfg(feature = "test-support")]
pub(super) fn paint_header(app: &mut App, ctx: &egui::Context, events: &[egui::Event]) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 480.0),
            )),
            events: events.to_vec(),
            ..Default::default()
        },
        |ui| {
            let rect =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(ui.available_width(), 40.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                app.window_header(ui);
            });
        },
    );
    output.textures_delta.clear();
}

pub(super) fn fixture() -> (App, egui::Context, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = false;
    let state = State {
        projects: ["a", "b"]
            .into_iter()
            .map(|id| Project {
                id: id.into(),
                name: id.into(),
                path: PathBuf::from(format!("/{id}")),
                layout: serde_json::Value::Null,
            })
            .collect(),
        selected_project: Some("a".into()),
        ..Default::default()
    };
    app.apply_state(state);
    (app, ctx, dir)
}

pub(super) fn session_fixture(sid: &str, kind: SessionKind) -> Session {
    Session {
        review: false,
        id: sid.into(),
        project_id: "a".into(),
        label: sid.into(),
        cwd: "/a".into(),
        kind,
        file: None,
        lifecycle: Lifecycle::Running,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "fixture".into(),
        pid: Some(42),
        truncated: false,
        cwd_confirmed: true,
    }
}

pub(super) fn poll_workspace_close_in_frame(app: &mut App, ctx: &egui::Context) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| app.poll_workspace_close(ui.ctx()),
    );
    output.textures_delta.clear();
}

pub(super) fn prompt_name(app: &App) -> Option<&str> {
    match &app.name_prompt {
        Some(
            workspace_ops::NamePrompt::File { name, .. }
            | workspace_ops::NamePrompt::Folder { name, .. }
            | workspace_ops::NamePrompt::Rename { name, .. },
        ) => Some(name),
        None => None,
    }
}

// Kept for upcoming native-rendering assertions; no test calls it yet.
#[allow(dead_code)]
pub(super) fn placeholder_center(
    shapes: &[egui::epaint::ClippedShape],
    needle: &str,
) -> Option<f32> {
    fn walk(shape: &egui::Shape, needle: &str) -> Option<f32> {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    if let Some(center) = walk(shape, needle) {
                        return Some(center);
                    }
                }
                None
            }
            egui::Shape::Text(text) if text.galley.text() == needle => {
                let bounds = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                Some(bounds.center().y)
            }
            _ => None,
        }
    }
    shapes
        .iter()
        .find_map(|clipped| walk(&clipped.shape, needle))
}

pub(super) fn paint_explorer(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let cwd = PathBuf::from("/a");
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 320.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.explorer_toolbar(ui, &cwd),
    );
    output.textures_delta.clear();
}

pub(super) fn key_press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

pub(super) fn assert_no_workspace_job(requests: &Receiver<Job>) {
    while let Ok(job) = requests.try_recv() {
        assert!(
            !matches!(job, Job::Workspace(..)),
            "explorer prompt must not submit"
        );
    }
}

pub(super) fn assert_rename_blocks(app: &App) {
    assert!(app.rename_session.is_some());
    assert!(!app.terminal_input_enabled("named"));
    assert!(!app.strip_terminal_input_enabled("named"));
    assert!(!app.shortcut_allowed("new_terminal"));
    assert!(app.browser_covered());
}

pub(super) fn assert_rename_draft_stays_live(app: &App) {
    assert_eq!(
        app.rename_session.as_ref().map(|(_, title)| title.as_str()),
        Some("Draft")
    );
    assert!(app.terminal_input_enabled("named"));
    assert!(app.strip_terminal_input_enabled("named"));
    assert!(app.shortcut_allowed("new_terminal"));
    assert!(!app.browser_covered());
}

pub(super) fn visible_ids(app: &App) -> Vec<String> {
    app.visible_projects()
        .into_iter()
        .map(|project| project.id.clone())
        .collect()
}

/// Pump `process_updates` until `project`'s activity stamp reaches
/// `at_least`. One pass drains at most 64 updates or 2 ms
/// (`ResultBudget`), so on a loaded machine a single pass may service
/// background completions first and leave the test's own update queued.
/// Production pumps every frame; tests must do the same.
pub(super) fn drain_until_project_activity(
    app: &mut App,
    ctx: &egui::Context,
    project: &str,
    at_least: u64,
) {
    for _ in 0..1_000 {
        app.process_updates(ctx);
        if app
            .preferences
            .project_activity
            .get(project)
            .is_some_and(|stamp| *stamp >= at_least)
        {
            return;
        }
    }
    panic!("timed out waiting for {project} activity to reach {at_least}");
}

/// One pass drains at most 64 updates or 2 ms, and startup completions
/// sit ahead of the test's own update. A loaded runner can defer that
/// update past a single call, so pump the way production pumps each frame.
pub(super) fn pump_until(app: &mut App, ctx: &egui::Context, mut ready: impl FnMut(&App) -> bool) {
    for _ in 0..100 {
        app.process_updates(ctx);
        if ready(app) {
            return;
        }
    }
    panic!("timed out waiting for a queued update");
}

/// Drive one headless frame with the IDE strip painted before the main
/// dock, matching `App::ui`, so a release is seen by both drop targets.
#[cfg(feature = "test-support")]
pub(super) fn cross_dock_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::Panel::bottom("ide-terminal")
                .resizable(false)
                .exact_size(180.0)
                .show(ui, |ui| app.ide_terminal_strip(ui));
            let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
            app.workspace_bar(ui, "a", &mut dock);
            app.paint_dock(ui, "a", &mut dock);
            app.layouts.insert("a".into(), dock);
            if app.pending_layout_save {
                app.pending_layout_save = false;
                app.note_focus_baselines();
            }
        },
    );
    output.textures_delta.clear();
}

#[cfg(feature = "test-support")]
pub(super) fn cross_glide(
    app: &mut App,
    ctx: &egui::Context,
    from: egui::Pos2,
    to: egui::Pos2,
    steps: usize,
) {
    for step in 1..=steps {
        // Glide step is a UI coordinate blend; the count can exceed the f32 mantissa.
        #[allow(clippy::cast_precision_loss)]
        let k = step as f32 / steps as f32;
        cross_dock_frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                from.x + (to.x - from.x) * k,
                from.y + (to.y - from.y) * k,
            ))],
        );
    }
}

/// Drive one headless frame of the strip plus the dock in real panel
/// order (strip first, dock after) with synthetic pointer events.
/// Needs test-support for the geometry records.
#[cfg(feature = "test-support")]
pub(super) fn combined_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
            app.workspace_bar(ui, "a", &mut dock);
            app.paint_dock(ui, "a", &mut dock);
            app.layouts.insert("a".into(), dock);
        },
    );
    output.textures_delta.clear();
}

#[cfg(feature = "test-support")]
pub(super) fn frame_center(app: &App, ctx: &egui::Context, name: &str) -> egui::Pos2 {
    let rect = app
        .fixture_rect(ctx, name)
        .unwrap_or_else(|| panic!("missing geometry for {name}"));
    egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)
}

#[cfg(feature = "test-support")]
pub(super) fn frame_press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

#[cfg(feature = "test-support")]
pub(super) fn frame_glide(
    app: &mut App,
    ctx: &egui::Context,
    from: egui::Pos2,
    to: egui::Pos2,
    steps: usize,
) {
    for step in 1..=steps {
        // Glide step is a UI coordinate blend; the count can exceed the f32 mantissa.
        #[allow(clippy::cast_precision_loss)]
        let k = step as f32 / steps as f32;
        combined_frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                from.x + (to.x - from.x) * k,
                from.y + (to.y - from.y) * k,
            ))],
        );
    }
}

pub(super) fn notice_fixture(
    id: &str,
    session: &str,
    state: AgentState,
    created: u64,
) -> Notification {
    Notification {
        id: id.into(),
        session_id: session.into(),
        invocation_id: id.into(),
        request_id: None,
        state,
        summary: state.label().into(),
        details: "Agent: codex\nEvent: PermissionRequest\nSession: test".into(),
        created,
        read: false,
        dismissed: false,
        resolved: false,
        snoozed_until: 0,
    }
}

#[cfg(feature = "test-support")]
pub(super) fn agent_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
}

#[cfg(feature = "test-support")]
pub(super) fn render_agents(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.agents_view(ui),
    );
    output.textures_delta.clear();
}

pub(super) fn generation_health(
    id: &str,
    directory: &std::path::Path,
    revision: u64,
    status: generations::Status,
) -> generations::Health {
    generations::Health {
        owner: generations::Generation {
            id: id.into(),
            data: directory.to_path_buf(),
            runtime: directory.to_path_buf(),
            version: "0.35.0".into(),
            build: "fixture".into(),
            protocol: 1,
            catalog: 1,
            status,
            pid: None,
        },
        revision,
        error: None,
        live_sessions: 1,
        capabilities: Vec::new(),
        helper: None,
    }
}

#[cfg(feature = "test-support")]
pub(super) fn click_agent_target(app: &mut App, ctx: &egui::Context, name: &str) {
    render_agents(app, ctx, vec![]);
    let pos = agent_target(ctx, name).unwrap().center();
    render_agents(app, ctx, vec![egui::Event::PointerMoved(pos)]);
    render_agents(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
    );
    render_agents(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }],
    );
}

#[cfg(feature = "test-support")]
pub(super) fn waiting_inbox() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
    app.state.notifications = vec![notice_fixture(
        "wait",
        "live-shell",
        AgentState::WaitingPermission,
        now(),
    )];
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    (app, ctx, dir, received)
}

#[cfg(feature = "test-support")]
pub(super) fn control_actions(received: &mpsc::Receiver<Job>) -> Vec<String> {
    let mut actions = Vec::new();
    while let Ok(job) = received.try_recv() {
        if let Job::Control(request, _) = job {
            match *request {
                Request::Notice { action, .. } => actions.push(action),
                Request::Focus { .. } => actions.push("focus".into()),
                _ => {}
            }
        }
    }
    actions
}
