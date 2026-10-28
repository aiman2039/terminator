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
    app.picker_active = false;
    // macOS shows the chooser; other platforms treat it as a no-op. Neither may panic.
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.first_project_dialog(ui.ctx());
    });
    output.textures_delta.clear();

    app.preferences.setup_completed = true;
    app.picker_active = false;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.first_project_dialog(ui.ctx());
    });
    output.textures_delta.clear();
}
