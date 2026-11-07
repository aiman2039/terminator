//! Pane zoom (one terminal or editor over the workspace) and floating
//! dock-back.
use super::*;

const SCREEN: egui::Vec2 = egui::vec2(1000.0, 600.0);
const PLACEHOLDER: &str = "Zoomed — click to restore";
const RECONNECTING: &str = "Reconnecting to session daemon…";

/// Project `a` with one shell `sid` in its dock, focused.
fn zoom_fixture(sid: &str) -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = fixture();
    app.state.sessions = vec![session_fixture(sid, SessionKind::Shell)];
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("main".into(), Tab::Terminal(sid.into()));
    app.selected = Some("a".into());
    app.active_session = Some(sid.into());
    (app, ctx, dir)
}

/// One headless frame of the center pane; the painted strings.
fn center_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            events,
            ..Default::default()
        },
        |ui| app.center_pane(ui),
    );
    output.textures_delta.clear();
    painted_strings(&output.shapes)
}

fn count(text: &[String], needle: &str) -> usize {
    text.iter().filter(|line| line.contains(needle)).count()
}

fn click_at(app: &mut App, ctx: &egui::Context, pos: egui::Pos2) {
    center_frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    center_frame(app, ctx, vec![button(true), button(false)]);
}

#[test]
fn zoomed_terminal_paints_once_per_frame() {
    let (mut app, ctx, _dir) = zoom_fixture("zoom-once");
    app.connected = false;
    let docked = center_frame(&mut app, &ctx, vec![]);
    assert_eq!(count(&docked, RECONNECTING), 1);
    app.run_shortcut(&ctx, Action::ZoomPane);
    assert!(app.pane_ops.zoomed.is_some());
    // The first frame sizes the overlay; the second shows it.
    center_frame(&mut app, &ctx, vec![]);
    let zoomed = center_frame(&mut app, &ctx, vec![]);
    // The dock leaves a placeholder and the overlay paints the body: one
    // body a frame, so the PTY never sees two sizes.
    assert_eq!(count(&zoomed, PLACEHOLDER), 1);
    assert_eq!(count(&zoomed, RECONNECTING), 1);
    app.run_shortcut(&ctx, Action::ZoomPane);
    assert!(app.pane_ops.zoomed.is_none());
    let restored = center_frame(&mut app, &ctx, vec![]);
    assert_eq!(count(&restored, PLACEHOLDER), 0);
    assert_eq!(count(&restored, RECONNECTING), 1);
}

#[test]
fn zoomed_editor_paints_once_per_frame() {
    let (mut app, ctx, dir) = fixture();
    // Absolute on every platform: the trust bar needs a file URI, and
    // Windows rejects `/a/...` as one.
    let path = dir.path().join("zoom-once.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("main".into(), Tab::NativeEditor { path: path.clone() });
    app.selected = Some("a".into());
    app.active_session = None;
    app.non_terminal_selected = true;
    center_frame(&mut app, &ctx, vec![]);
    let docked = center_frame(&mut app, &ctx, vec![]);
    // The editor's trust bar marks each painted editor body.
    let body = count(&docked, "for language servers");
    assert_eq!(body, 1, "editor painted once: {docked:?}");
    app.run_shortcut(&ctx, Action::ZoomPane);
    let zoomed = app.pane_ops.zoomed.clone().expect("editor zoomed");
    assert_eq!(zoomed.tab, Tab::NativeEditor { path: path.clone() });
    assert_eq!(app.native_focus_pending, Some(zoomed.source));
    center_frame(&mut app, &ctx, vec![]);
    let text = center_frame(&mut app, &ctx, vec![]);
    assert_eq!(count(&text, PLACEHOLDER), 1);
    assert_eq!(count(&text, "for language servers"), body);
}

#[test]
fn margin_click_unzooms_and_restores_focus() {
    let (mut app, ctx, _dir) = zoom_fixture("zoom-margin");
    app.connected = false;
    app.run_shortcut(&ctx, Action::ZoomPane);
    center_frame(&mut app, &ctx, vec![]);
    // Inside the card nothing happens.
    click_at(&mut app, &ctx, egui::pos2(500.0, 300.0));
    assert!(app.pane_ops.zoomed.is_some());
    // The dimmed margin around it restores the pane, focus included.
    click_at(&mut app, &ctx, egui::pos2(3.0, 3.0));
    assert!(app.pane_ops.zoomed.is_none());
    assert_eq!(app.active_session.as_deref(), Some("zoom-margin"));
    assert!(!app.non_terminal_selected);
}

#[test]
fn escape_stays_with_the_zoomed_terminal() {
    let (mut app, ctx, _dir) = zoom_fixture("zoom-escape");
    app.connected = false;
    app.run_shortcut(&ctx, Action::ZoomPane);
    center_frame(&mut app, &ctx, vec![]);
    center_frame(&mut app, &ctx, vec![key_press(egui::Key::Escape)]);
    assert!(app.pane_ops.zoomed.is_some());
}

#[test]
fn zoom_ends_with_its_session_tab_or_a_center_page() {
    let (mut app, ctx, _dir) = zoom_fixture("zoom-end");
    app.connected = false;
    app.run_shortcut(&ctx, Action::ZoomPane);
    app.settings_open = true;
    center_frame(&mut app, &ctx, vec![]);
    assert!(app.pane_ops.zoomed.is_none(), "Settings ends the zoom");
    app.settings_open = false;
    app.run_shortcut(&ctx, Action::ZoomPane);
    app.state.sessions[0].lifecycle = Lifecycle::Ended;
    center_frame(&mut app, &ctx, vec![]);
    assert!(app.pane_ops.zoomed.is_none(), "an ended session ends it");
    app.state.sessions[0].lifecycle = Lifecycle::Running;
    app.run_shortcut(&ctx, Action::ZoomPane);
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("other".into(), Tab::Terminal("x".into()));
    center_frame(&mut app, &ctx, vec![]);
    assert!(app.pane_ops.zoomed.is_none(), "another tab ends it");
}

#[test]
fn header_zoom_request_zooms_that_pane_and_a_second_restores() {
    let (mut app, ctx, _dir) = zoom_fixture("zoom-header");
    app.connected = false;
    let tab = Tab::Terminal("zoom-header".into());
    app.queue_zoom(Some(tab.clone()), None);
    center_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.pane_ops.zoomed.as_ref().map(|z| &z.tab), Some(&tab));
    app.queue_zoom(Some(tab), None);
    assert!(app.pane_ops.zoomed.is_none());
}

#[test]
fn only_terminals_and_editors_zoom() {
    assert!(App::zoomable(&Tab::Terminal("one".into())));
    assert!(App::zoomable(&Tab::NativeEditor {
        path: "/a/note.md".into()
    }));
    assert!(!App::zoomable(&Tab::browser_file("/a/page.html".into())));
    assert!(!App::zoomable(&Tab::Player));
}

fn floating_frame(app: &mut App, ctx: &egui::Context) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            ..Default::default()
        },
        |ui| app.paint_floating(ui.ctx()),
    );
    output.textures_delta.clear();
}

#[test]
fn dock_back_single_pane_returns_to_home_leaf_and_keeps_window_for_others() {
    let (mut app, ctx, _dir) = fixture();
    app.connected = false;
    let home = app.layouts["a"].tabs[0].id.clone();
    let first = Tab::Terminal("float-home-1".into());
    let second = Tab::Terminal("float-home-2".into());
    app.floating.push(float_window(FloatWindowInput {
        name: "float-dock-back",
        project: "a",
        home: &home,
        tabs: vec![first.clone(), second.clone()],
    }));
    let viewport = app.floating[0].viewport;
    app.pane_ops.float_return = Some((viewport, Some(first.clone())));
    floating_frame(&mut app, &ctx);
    let window = app
        .floating
        .first()
        .expect("window kept for the other pane");
    let dock = window.dock.as_ref().expect("dock kept");
    assert!(dock.find_tab(&first).is_none());
    assert!(dock.find_tab(&second).is_some());
    let workspace = &app.layouts["a"];
    assert!(workspace.find_tab(&first).is_some());
    assert_eq!(workspace.active, home);
    // Dock back all takes the rest home and closes the window.
    app.pane_ops.float_return = Some((viewport, None));
    floating_frame(&mut app, &ctx);
    assert!(app.floating.is_empty());
    assert!(app.layouts["a"].contains(&second));
}

#[test]
fn dock_back_of_the_last_pane_closes_the_window() {
    let (mut app, ctx, _dir) = fixture();
    app.connected = false;
    let home = app.layouts["a"].tabs[0].id.clone();
    let only = Tab::Terminal("float-only".into());
    app.floating.push(float_window(FloatWindowInput {
        name: "float-last",
        project: "a",
        home: &home,
        tabs: vec![only.clone()],
    }));
    let viewport = app.floating[0].viewport;
    app.pane_ops.float_return = Some((viewport, Some(only.clone())));
    floating_frame(&mut app, &ctx);
    assert!(app.floating.is_empty());
    assert!(app.layouts["a"].contains(&only));
}
