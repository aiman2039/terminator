use super::*;

#[test]
fn moved_maps_the_path_and_everything_under_it() {
    let (from, to) = (Path::new("/r/a"), Path::new("/r/b"));
    assert_eq!(
        moved(Path::new("/r/a"), from, to),
        Some(PathBuf::from("/r/b"))
    );
    assert_eq!(
        moved(Path::new("/r/a/x.rs"), from, to),
        Some(PathBuf::from("/r/b/x.rs"))
    );
    assert_eq!(moved(Path::new("/r/ab"), from, to), None);
}

#[test]
fn clipboard_events_reach_the_tree_as_copy_cut_and_paste_keys() {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![
            egui::Event::Copy,
            egui::Event::Cut,
            egui::Event::Paste(String::new()),
        ],
        ..Default::default()
    };
    let mut keys = Vec::new();
    let mut output = ctx.run_ui(raw, |ui| keys = App::tree_key_events(ui).0);
    output.textures_delta.clear();
    let command = egui::Modifiers::COMMAND;
    assert_eq!(
        keys,
        vec![
            (egui::Key::C, command),
            (egui::Key::X, command),
            (egui::Key::V, command)
        ]
    );
}

#[test]
fn a_copy_the_tree_handled_does_not_reach_the_terminal() {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::Copy, egui::Event::Text("x".into())],
        ..Default::default()
    };
    let mut left = Vec::new();
    let mut output = ctx.run_ui(raw, |ui| {
        let keys = App::tree_key_events(ui).0;
        ui.input_mut(|input| App::consume_tree_keys(input, &keys));
        left = ui.input(|input| input.events.clone());
    });
    output.textures_delta.clear();
    assert_eq!(left, vec![egui::Event::Text("x".into())]);
}
