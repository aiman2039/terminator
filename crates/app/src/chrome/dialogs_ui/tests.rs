use super::*;
use terminator_core::Paths;

#[test]
fn first_project_dialog_renders_only_before_setup_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = false;
    app.preferences.setup_completed = false;
    app.state_loaded = true;
    app.picker.close();
    // macOS shows the chooser; other platforms treat it as a no-op. Neither may panic.
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.first_project_dialog(ui.ctx());
    });
    output.textures_delta.clear();

    app.preferences.setup_completed = true;
    app.picker.close();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.first_project_dialog(ui.ctx());
    });
    output.textures_delta.clear();
}

fn paint_restart_confirm(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        app.popups.begin_frame(ui.ctx());
        app.restart_confirm_dialog(ui.ctx());
        app.popups.end_frame();
    });
    output.textures_delta.clear();
}

/// The restart confirmation opens from the status bar while a terminal
/// keeps focus: its keys must answer the dialog and never reach the PTY.
#[test]
fn restart_confirmation_stops_terminal_input_and_escape_cancels_it() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.service_op = ServiceOp::ConfirmingRestart;
    paint_restart_confirm(&mut app, &ctx, Vec::new());
    paint_restart_confirm(&mut app, &ctx, Vec::new());
    assert!(app.popups.confirm_open());
    assert!(!app.terminal_copy_enabled("shell"));
    assert!(!app.strip_terminal_copy_enabled("shell"));
    assert!(!app.float_terminal_copy_enabled("shell"));
    paint_restart_confirm(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(app.service_op, ServiceOp::Idle);
    // The frame that answered still showed it; the next one no longer does.
    paint_restart_confirm(&mut app, &ctx, Vec::new());
    paint_restart_confirm(&mut app, &ctx, Vec::new());
    assert!(!app.popups.confirm_open());
}

/// Enter in the restart confirmation is Cancel: stopping every session is
/// destructive, so it is never the default.
#[test]
fn enter_in_the_restart_confirmation_never_stops_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.service_op = ServiceOp::ConfirmingRestart;
    paint_restart_confirm(&mut app, &ctx, Vec::new());
    paint_restart_confirm(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(app.service_op, ServiceOp::Idle);
}
