use super::*;

#[test]
fn perforce_panel_renders_changelists_and_confirms_submit() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let context = ContextData {
        cwd: "/repo".into(),
        root: Some("/repo".into()),
        git_dirs: Vec::new(),
        perforce: Some(crate::perforce::Workspace {
            client: "test-client".into(),
            command_root: "/repo".into(),
            stream: "//streams/main".into(),
            pending: vec![("42".into(), "Fix file".into())],
            history: vec![("41".into(), "Initial".into())],
            files: vec![crate::perforce::File {
                path: "/repo/changed.rs".into(),
                action: "edit".into(),
                change: "42".into(),
                opened: true,
            }],
        }),
        branch: String::new(),
        changes: Vec::new(),
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    let draw = |events, confirm| {
        let mut outcome = GitPanelOutcome::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let id = ui.make_persistent_id(("perforce", &context.root));
                ui.data_mut(|data| {
                    data.insert_temp(id, "42".to_string());
                    if confirm {
                        data.insert_temp(id.with("submit"), "42".to_string());
                    }
                });
                outcome = panel(
                    ui,
                    PerforcePanelInput {
                        context: &context,
                        theme: &theme,
                        open_shortcut: "",
                    },
                );
            },
        );
        output.textures_delta.clear();
        (output, outcome)
    };
    draw(Vec::new(), false);
    let (output, outcome) = draw(Vec::new(), false);
    assert!(
        outcome.perforce.is_empty(),
        "rendering must not mutate Perforce"
    );
    let text = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some((text.pos, text.galley.text().to_string()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let painted = text(&output);
    assert!(painted.iter().any(|(_, text)| text == "Perforce"));
    assert!(
        painted
            .iter()
            .any(|(_, text)| text.contains("Changelist 42"))
    );
    assert!(!painted.iter().any(|(_, text)| text == "Amend"));
    let (output, outcome) = draw(Vec::new(), true);
    assert!(outcome.perforce.is_empty());
    let button = text(&output)
        .into_iter()
        .find(|(_, text)| text == "Submit")
        .unwrap()
        .0
        + egui::vec2(3.0, 3.0);
    draw(
        vec![
            egui::Event::PointerMoved(button),
            egui::Event::PointerButton {
                pos: button,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        false,
    );
    let (_, outcome) = draw(
        vec![egui::Event::PointerButton {
            pos: button,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        false,
    );
    assert!(matches!(outcome.perforce.as_slice(), [Op::Submit(number)] if number == "42"));
}
