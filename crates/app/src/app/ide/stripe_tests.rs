use crate::*;

#[test]
fn more_opens_unpinned_tools_on_either_side() {
    use crate::app::tests::support::{fixture, frame_press};
    for right in [false, true] {
        let (mut app, ctx, _dir) = fixture();
        let draw = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.tool_stripe(ui, right),
            );
            output.textures_delta.clear();
        };
        draw(&mut app, vec![]);
        draw(&mut app, vec![]);
        let name = format!("stripe-{}-More", if right { "right" } else { "left" });
        let more = diagnostics::current_rect(&ctx, &name).unwrap().center();
        for events in [
            vec![egui::Event::PointerMoved(more)],
            vec![frame_press(more, true)],
            vec![frame_press(more, false)],
            vec![],
            vec![],
        ] {
            draw(&mut app, events);
        }
        for (_, _, label) in super::STRIPE_TOOLS {
            assert!(
                diagnostics::current_rect(&ctx, label).is_some(),
                "More contains {label}"
            );
        }
        // Structure is pinned on neither side by default.
        let structure = diagnostics::current_rect(&ctx, "Structure")
            .unwrap()
            .center();
        for events in [
            vec![egui::Event::PointerMoved(structure)],
            vec![frame_press(structure, true)],
            vec![frame_press(structure, false)],
        ] {
            draw(&mut app, events);
        }
        assert!(!app.stripe_pinned(right, "Structure"));
        if right {
            assert!(app.preferences.visible);
            assert_eq!(app.preferences.tool, SidebarTool::Structure);
        } else {
            assert!(app.preferences.left_visible);
            assert_eq!(
                app.preferences.left_content(),
                LeftContent::Tool(SidebarTool::Structure)
            );
        }
    }
}

#[test]
fn tool_pins_survive_restart_and_do_not_change_sidebar_placement() {
    let (mut app, _ctx, dir) = crate::app::tests::support::fixture();
    let left = app.preferences.left_content();
    let right = app.preferences.tool;
    assert!(app.stripe_pinned(false, "Projects"));
    assert!(!app.stripe_pinned(false, "History"));
    app.set_stripe_pin(false, "History", true);
    app.set_stripe_pin(false, "History", true);
    app.set_stripe_pin(false, "Git", false);
    app.set_stripe_pin(true, "Hosts", true);
    app.preferences.save(dir.path()).unwrap();
    let loaded = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(loaded.left_content(), left);
    assert_eq!(loaded.tool, right);
    assert_eq!(
        loaded
            .left_tool_pins
            .iter()
            .filter(|pin| *pin == "History")
            .count(),
        1
    );
    assert!(!loaded.left_tool_pins.iter().any(|pin| pin == "Git"));
    assert_eq!(
        loaded.right_tool_pins,
        ["Explorer", "Git", "History", "Info", "Hosts"]
    );
    let old: UiPreferences = serde_json::from_str("{}").unwrap();
    assert!(old.left_tool_pins.iter().any(|pin| pin == "Projects"));
}

#[test]
fn an_empty_right_stripe_gets_the_header_tools_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ui-preferences.json");
    std::fs::write(&file, r#"{"version":2,"right_tool_pins":[]}"#).unwrap();
    let mut loaded = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(
        loaded.right_tool_pins,
        ["Explorer", "Git", "History", "Info"]
    );
    // A later empty list is the user's choice and stays empty.
    loaded.right_tool_pins.clear();
    loaded.save(dir.path()).unwrap();
    assert!(
        UiPreferences::load(dir.path())
            .unwrap()
            .right_tool_pins
            .is_empty()
    );
}

#[test]
fn only_the_left_stripe_has_a_search_button() {
    let (mut app, ctx, _dir) = crate::app::tests::support::fixture();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            app.tool_stripe(ui, false);
            app.tool_stripe(ui, true);
        },
    );
    output.textures_delta.clear();
    assert!(diagnostics::current_rect(&ctx, "stripe-left-Search").is_some());
    assert!(diagnostics::current_rect(&ctx, "stripe-right-Search").is_none());
    assert!(diagnostics::current_rect(&ctx, "stripe-right-Git").is_some());
}

#[test]
fn short_tool_stripes_clip_tools_above_the_search_button() {
    let (mut app, ctx, _dir) = crate::app::tests::support::fixture();
    for height in [650.0, 220.0, 90.0, 400.0, 70.0, 650.0] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, height),
                )),
                ..Default::default()
            },
            |ui| app.tool_stripe(ui, false),
        );
        output.textures_delta.clear();
        let search = ctx.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", "stripe-left-Search")))
                .expect("Search remains available")
        });
        assert!(search.bottom() <= height);
        assert_eq!(search.height(), 40.0);
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape
                && matches!(text.galley.text(), "VNC" | "Structure")
            {
                assert!(
                    clipped.clip_rect.bottom() <= search.top(),
                    "height={height}"
                );
            }
        }
    }
}
