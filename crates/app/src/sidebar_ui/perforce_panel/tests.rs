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

#[test]
fn a_running_perforce_action_shows_progress_and_cancel_stops_it() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let context = ContextData {
        cwd: "/repo".into(),
        root: Some("/repo".into()),
        git_dirs: Vec::new(),
        perforce: Some(crate::perforce::Workspace {
            client: "test-client".into(),
            command_root: "/repo".into(),
            stream: String::new(),
            pending: Vec::new(),
            history: Vec::new(),
            files: Vec::new(),
        }),
        branch: String::new(),
        changes: Vec::new(),
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    let mut slot = crate::perforce::RunSlot::default();
    let run = slot.start(Op::Sync).unwrap();
    publish(&ctx, slot.progress());
    let draw = |events| {
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
                let _ = panel(
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
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.pos, text.galley.text().to_string())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    draw(Vec::new());
    let painted = draw(Vec::new());
    assert!(painted.iter().any(|(_, text)| text == "Running p4 sync…"));
    let button =
        painted.iter().find(|(_, text)| text == "Cancel").unwrap().0 + egui::vec2(3.0, 3.0);
    draw(vec![
        egui::Event::PointerMoved(button),
        egui::Event::PointerButton {
            pos: button,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    draw(vec![egui::Event::PointerButton {
        pos: button,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(run.cancel.is_cancelled(), "Cancel stops the p4 job");
    let painted = draw(Vec::new());
    assert!(painted.iter().any(|(_, text)| text == "Stopping p4 sync…"));
    slot.finish();
    publish(&ctx, slot.progress());
    let painted = draw(Vec::new());
    assert!(!painted.iter().any(|(_, text)| text.contains("p4 sync")));
}

#[test]
fn perforce_actions_use_bundled_icons() {
    for name in [
        "RefreshCw",
        "Download",
        "Upload",
        "Archive",
        "ArchiveRestore",
    ] {
        assert!(
            crate::icons::is_ui_icon(name),
            "{name} falls back to file.svg"
        );
    }
}
