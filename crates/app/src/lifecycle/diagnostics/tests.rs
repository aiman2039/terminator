use super::*;

fn render(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
    let mut output = ctx.run_ui(egui::RawInput::default(), draw);
    output.textures_delta.clear();
}

fn click_save() -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.path = Some("fixture.png".into());
    diagnostics.actions = serde_json::from_value(serde_json::json!([
        {"at_ms":0,"target":"Save and close"}
    ]))
    .unwrap();
    diagnostics
}

#[test]
fn late_fixture_actions_keep_their_planned_settling_intervals() {
    let mut diagnostics = click_save();
    diagnostics.actions = serde_json::from_value(serde_json::json!([
        {"at_ms":900,"target":"tool-Git"},
        {"at_ms":1200,"target":"header-overflow"},
        {"at_ms":1400,"target":"settings"},
        {"at_ms":2000,"target":"settings-section:Terminal & Editor"}
    ]))
    .unwrap();
    let first = &diagnostics.actions[0];
    assert!(!diagnostics.action_ready(first, Duration::from_millis(899)));
    assert!(diagnostics.action_ready(first, Duration::from_secs(3)));
    diagnostics.last_action = Some((900, Duration::from_secs(3)));
    let menu = &diagnostics.actions[1];
    assert!(!diagnostics.action_ready(menu, Duration::from_millis(3016)));
    assert!(diagnostics.action_ready(menu, Duration::from_millis(3300)));
    diagnostics.last_action = Some((1200, Duration::from_secs(4)));
    let settings = &diagnostics.actions[2];
    assert!(!diagnostics.action_ready(settings, Duration::from_millis(4016)));
    assert!(diagnostics.action_ready(settings, Duration::from_millis(4200)));
    diagnostics.last_action = Some((1400, Duration::from_millis(4200)));
    let section = &diagnostics.actions[3];
    assert!(!diagnostics.action_ready(section, Duration::from_millis(4216)));
    assert!(diagnostics.action_ready(section, Duration::from_millis(4800)));
}

#[test]
fn on_time_and_equal_time_fixture_actions_keep_their_schedule() {
    let mut diagnostics = click_save();
    diagnostics.last_action = Some((0, Duration::ZERO));
    assert!(diagnostics.action_ready(&diagnostics.actions[0], Duration::ZERO));
    diagnostics.actions[0].at_ms = 200;
    assert!(!diagnostics.action_ready(&diagnostics.actions[0], Duration::from_millis(199)));
    assert!(diagnostics.action_ready(&diagnostics.actions[0], Duration::from_millis(200)));
    // Dynamic scripts may append an action whose timestamp is already past.
    diagnostics.last_action = Some((500, Duration::from_millis(900)));
    assert!(diagnostics.action_ready(&diagnostics.actions[0], Duration::from_millis(900)));
}

#[test]
fn hidden_control_does_not_consume_a_fixture_click() {
    let ctx = egui::Context::default();
    let mut diagnostics = click_save();
    render(&ctx, |ui| {
        let response = ui.button("Save and close");
        record(ui.ctx(), "Save and close", response.rect);
    });
    assert!(current_rect(&ctx, "Save and close").is_some());
    // Cancel has hidden the prompt; its saved rectangle must not be used.
    render(&ctx, |_| {});
    assert!(current_rect(&ctx, "Save and close").is_none());
    let mut input = egui::RawInput::default();
    diagnostics.input(&ctx, &mut input);
    assert_eq!(diagnostics.actions_completed(), 0);
    assert!(
        !input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerButton { pressed: true, .. }))
    );
}

#[test]
fn disabled_control_waits_until_enabled_before_fixture_click() {
    let ctx = egui::Context::default();
    let mut diagnostics = click_save();
    for enabled in [true, false] {
        render(&ctx, |ui| {
            let response = ui.add_enabled(enabled, egui::Button::new("Save and close"));
            record_response(ui.ctx(), "Save and close", &response);
        });
    }
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    assert_eq!(diagnostics.actions_completed(), 0);
    render(&ctx, |ui| {
        let response = ui.button("Save and close");
        record_response(ui.ctx(), "Save and close", &response);
    });
    let mut input = egui::RawInput::default();
    diagnostics.input(&ctx, &mut input);
    assert_eq!(diagnostics.actions_completed(), 1);
    assert!(
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerButton { pressed: true, .. }))
    );
}

#[test]
fn sizing_pass_rows_wait_for_the_real_popup_layout() {
    let ctx = egui::Context::default();
    let mut diagnostics = click_save();
    // The first popup pass is invisible and non-interactable; a click on
    // its provisional rect could never reach the row.
    render(&ctx, |ui| {
        let mut sizing = ui.new_child(egui::UiBuilder::new().sizing_pass());
        let response = sizing.button("Save and close");
        record_ui(&sizing, "Save and close", response.rect);
    });
    assert!(current_rect(&ctx, "Save and close").is_none());
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    assert_eq!(diagnostics.actions_completed(), 0);
    render(&ctx, |ui| {
        let response = ui.button("Save and close");
        record_ui(ui, "Save and close", response.rect);
    });
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    assert_eq!(diagnostics.actions_completed(), 1);
}

#[test]
fn capture_waits_for_the_final_layout_pass() {
    let ctx = egui::Context::default();
    let mut diagnostics = Diagnostics::default();
    diagnostics.path = Some("fixture.png".into());
    diagnostics.started = Instant::now()
        .checked_sub(Duration::from_secs(4))
        .unwrap_or_else(Instant::now);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("fixture sizing pass");
            diagnostics.capture(ui.ctx());
            assert!(!diagnostics.requested);
        } else {
            diagnostics.capture(ui.ctx());
            assert!(diagnostics.requested);
        }
    });
    output.textures_delta.clear();
    assert!(diagnostics.requested);
}
#[test]
fn desktop_typing_cannot_enter_an_isolated_fixture() {
    let mut diagnostics = Diagnostics::default();
    diagnostics.path = Some("fixture.png".into());
    let mut input = egui::RawInput {
        events: vec![
            egui::Event::Text("unrelated desktop input".into()),
            egui::Event::Paste("clipboard".into()),
        ],
        ..Default::default()
    };
    diagnostics.input(&egui::Context::default(), &mut input);
    assert!(input.events.is_empty());
}
