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
    // The pointer arrives on this frame and presses on the next.
    assert!(!pressed(&input));
    let mut input = egui::RawInput::default();
    diagnostics.input(&ctx, &mut input);
    assert!(pressed(&input));
}

fn pressed(input: &egui::RawInput) -> bool {
    input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::PointerButton { pressed: true, .. }))
}

#[test]
fn a_moved_pointer_presses_one_frame_later_and_a_still_one_at_once() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"at_ms":0,"target":"Save and close"},
        {"at_ms":0,"target":"Save and close"}
    ]));
    let mut frames = Vec::new();
    for _ in 0..6 {
        frames.push(frame_with_button(&ctx, &mut diagnostics, 0.0));
    }
    // Move, press, release, then the second click at the same place.
    assert_eq!(frames, [false, true, false, true, false, false]);
    assert_eq!(diagnostics.actions_completed(), 2);
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

fn waiting_script(actions: serde_json::Value) -> Diagnostics {
    let mut diagnostics = click_save();
    diagnostics.actions = serde_json::from_value(actions).unwrap();
    // Past the startup grace, so only the wait conditions decide.
    diagnostics.started = Instant::now()
        .checked_sub(Duration::from_secs(4))
        .unwrap_or_else(Instant::now);
    diagnostics
}

#[test]
fn hidden_popup_target_refires_the_previous_step_a_bounded_number_of_times() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"Save and close","right_click":true,"wait":{}},
        {"target":"Open changes","wait":{"retry_previous_ms":0}}
    ]));
    // The popup row never shows, so every right-click counts as lost.
    let presses = (0..200)
        .filter(|_| frame_with_button(&ctx, &mut diagnostics, 0.0))
        .count();
    assert_eq!(presses, 4, "first click plus three retries");
    assert_eq!(diagnostics.actions_completed(), 1);
}

#[test]
fn waiting_action_does_not_fire_during_startup_settle() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"Save and close","wait":{}}
    ]));
    diagnostics.started = Instant::now();
    for _ in 0..5 {
        assert!(!frame_with_button(&ctx, &mut diagnostics, 0.0));
    }
    assert_eq!(diagnostics.actions_completed(), 0);
}

/// Renders "Save and close" at `offset`, then injects the next frame's input.
fn frame_with_button(ctx: &egui::Context, diagnostics: &mut Diagnostics, offset: f32) -> bool {
    render(ctx, |ui| {
        ui.add_space(offset);
        let response = ui.button("Save and close");
        record(ui.ctx(), "Save and close", response.rect);
    });
    let mut input = egui::RawInput::default();
    diagnostics.input(ctx, &mut input);
    input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::PointerButton { pressed: true, .. }))
}

#[test]
fn waiting_action_fires_when_target_holds_still_not_at_its_time() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"at_ms":600000,"target":"Save and close","wait":{}}
    ]));
    assert!(!frame_with_button(&ctx, &mut diagnostics, 0.0));
    assert!(!frame_with_button(&ctx, &mut diagnostics, 0.0));
    assert!(!frame_with_button(&ctx, &mut diagnostics, 0.0));
    assert_eq!(diagnostics.actions_completed(), 1);
    assert!(frame_with_button(&ctx, &mut diagnostics, 0.0));
}

#[test]
fn moving_target_restarts_the_stability_wait() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"Save and close","wait":{}}
    ]));
    for offset in [0.0, 10.0, 20.0, 30.0, 40.0] {
        assert!(!frame_with_button(&ctx, &mut diagnostics, offset));
    }
    assert!(!frame_with_button(&ctx, &mut diagnostics, 40.0));
    assert!(!frame_with_button(&ctx, &mut diagnostics, 40.0));
    assert_eq!(diagnostics.actions_completed(), 1);
    assert!(frame_with_button(&ctx, &mut diagnostics, 40.0));
}

#[test]
fn waiting_action_needs_present_and_absent_targets() {
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"Save and close","wait":{"present":["diff-undo"],"absent":["popup"],"stable_frames":1}}
    ]));
    assert!(!frame_with_button(&ctx, &mut diagnostics, 0.0));
    let mut draw = |undo: bool, popup: bool| {
        render(&ctx, |ui| {
            let response = ui.button("Save and close");
            record(ui.ctx(), "Save and close", response.rect);
            if undo {
                record(ui.ctx(), "diff-undo", response.rect);
            }
            if popup {
                record(ui.ctx(), "popup", response.rect);
            }
        });
        diagnostics.input(&ctx, &mut egui::RawInput::default());
        diagnostics.actions_completed()
    };
    assert_eq!(draw(true, true), 0);
    assert_eq!(draw(true, false), 1);
}

#[test]
fn check_action_ends_the_case_once_its_file_state_is_reached() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("done");
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"fixture-done","check":true,"capture":true,
         "wait":{"file":{"path":marker,"contains":"ready"}}}
    ]));
    render(&ctx, |_| {});
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    std::fs::write(&marker, "loading").unwrap();
    render(&ctx, |_| {});
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    assert_eq!(diagnostics.actions_completed(), 0);
    assert!(!diagnostics.requested);
    std::fs::write(&marker, "ready").unwrap();
    render(&ctx, |_| {});
    let mut input = egui::RawInput::default();
    diagnostics.input(&ctx, &mut input);
    assert_eq!(diagnostics.actions_completed(), 1);
    assert!(diagnostics.requested, "The final check must end the case");
    assert!(input.events.is_empty(), "A check action sends no input");
}

#[test]
fn file_wait_matches_a_json_pointer_value() {
    let dir = tempfile::tempdir().unwrap();
    let prefs = dir.path().join("ui-preferences.json");
    let ctx = egui::Context::default();
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"saved","check":true,
         "wait":{"file":{"path":prefs,"pointer":"/tool","value":"history"}}}
    ]));
    for (text, completed) in [
        ("not json", 0),
        (r#"{"tool":"Git"}"#, 0),
        (r#"{"tool":"history"}"#, 1),
    ] {
        std::fs::write(&prefs, text).unwrap();
        render(&ctx, |_| {});
        diagnostics.input(&ctx, &mut egui::RawInput::default());
        assert_eq!(diagnostics.actions_completed(), completed, "{text}");
    }
}

#[test]
fn waiting_action_keeps_its_settle_time_after_the_previous_step() {
    let mut diagnostics = waiting_script(serde_json::json!([
        {"target":"saved","check":true,"wait":{"settle_ms":60000}}
    ]));
    let ctx = egui::Context::default();
    diagnostics.last_action = Some((0, diagnostics.started.elapsed()));
    render(&ctx, |_| {});
    diagnostics.input(&ctx, &mut egui::RawInput::default());
    assert_eq!(diagnostics.actions_completed(), 0);
}

#[test]
fn misspelled_wait_condition_is_rejected() {
    let parsed = serde_json::from_value::<Vec<FixtureAction>>(serde_json::json!([
        {"target":"Save and close","wait":{"presnt":["diff-undo"]}}
    ]));
    assert!(parsed.is_err());
}
