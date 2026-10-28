use super::*;

#[test]
fn installation_shows_only_active_live_or_actionable_service_owners() {
    use terminator_core::generations::{Generation, Health, Status};
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(directory.path().into()));
    app.connected = true;
    app.state.generation = "current".into();
    app.state.attachment_helper_available = Some(true);
    app.state.capabilities = vec![STABLE_HELPER_CAPABILITY.into()];
    let health = |id: &str, status, live_sessions, error| Health {
        owner: Generation {
            id: id.into(),
            version: id.into(),
            status,
            data: directory.path().join(id),
            runtime: directory.path().join(id).join("run"),
            build: "fixture".into(),
            protocol: 1,
            catalog: 1,
            pid: None,
        },
        live_sessions,
        error,
        revision: 0,
        capabilities: vec![],
        helper: None,
    };
    app.state.generations = vec![
        health("current", Status::Active, 0, None),
        health("older-live", Status::Draining, 2, None),
        health("retired-empty", Status::Retired, 0, None),
        health("draining-empty", Status::Draining, 0, None),
        health("prepared-empty", Status::Prepared, 0, None),
        health(
            "unavailable",
            Status::Draining,
            0,
            Some("Saved state unavailable".into()),
        ),
    ];
    fn text(shape: &egui::Shape, output: &mut String) {
        match shape {
            egui::Shape::Text(shape) => {
                output.push_str(shape.galley.text());
                output.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, output);
                }
            }
            _ => {}
        }
    }
    let mut painted = String::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| app.installation_settings(ui),
        );
        painted.clear();
        for shape in &output.shapes {
            text(&shape.shape, &mut painted);
        }
        output.textures_delta.clear();
    }
    assert!(painted.contains("current · Active · 0 live sessions"));
    assert!(painted.contains("older-live · Draining · 2 live sessions"));
    assert!(painted.contains("unavailable · Draining · session status unavailable"));
    for hidden in ["retired-empty", "draining-empty", "prepared-empty"] {
        assert!(!painted.contains(hidden));
    }
    assert_eq!(
        app.state.generations.len(),
        6,
        "filtering must not remove saved owners"
    );
}
