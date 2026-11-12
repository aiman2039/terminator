use super::*;

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One frame of mode input; returns the keyboard events left for the
/// terminal widget that paints after it.
fn frame(app: &mut App, ctx: &egui::Context, sid: &str, events: Vec<egui::Event>) -> usize {
    let mut left = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            app.key_mode_input(ui.ctx(), sid);
            left = ui.input(|i| {
                i.events
                    .iter()
                    .filter(|e| {
                        matches!(
                            e,
                            egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)
                        )
                    })
                    .count()
            });
        },
    );
    output.textures_delta.clear();
    left
}

fn shell(app: &mut App, ctx: &egui::Context) -> String {
    let sid = "modes-shell".to_owned();
    let backend = egui_term::TerminalBackend::new(
        901,
        ctx.clone(),
        app.pty_tx.clone(),
        egui_term::BackendSettings {
            shell: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf 'see src/main.rs and https://example.com/x\\r\\n'; sleep 30".into(),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    app.backends.insert(sid.clone(), backend);
    app.focus_terminal(sid.clone());
    let started = Instant::now();
    loop {
        let backend = app.backends.get_mut(&sid).unwrap();
        backend.sync();
        if backend.visible_targets().len() >= 2 {
            return sid;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "text never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn hints_mode_keys_never_reach_the_pty() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    let sid = shell(&mut app, &ctx);
    app.toggle_hints_mode();
    let Some(KeyMode::Hints(hints)) = app.key_modes.on(&sid) else {
        panic!("hints mode is on");
    };
    let labels: Vec<_> = hints.items.iter().map(|(_, t)| t.text.clone()).collect();
    assert_eq!(labels, ["src/main.rs", "https://example.com/x"]);
    let typed = vec![press(egui::Key::X), egui::Event::Text("x".into())];
    assert_eq!(frame(&mut app, &ctx, &sid, typed), 0);
    assert!(app.key_modes.on(&sid).is_some());
    assert_eq!(
        frame(&mut app, &ctx, &sid, vec![press(egui::Key::Escape)]),
        0
    );
    assert!(app.key_modes.on(&sid).is_none());
}

#[test]
fn copy_mode_keys_never_reach_the_pty_and_y_copies_the_selection() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    let sid = shell(&mut app, &ctx);
    app.toggle_copy_mode();
    assert!(app.backends[&sid].copy_mode());
    let keys = ["g", "g", "V", "y"]
        .into_iter()
        .map(|c| egui::Event::Text(c.into()))
        .chain([egui::Event::Paste("rm -rf /".into())])
        .collect();
    assert_eq!(frame(&mut app, &ctx, &sid, keys), 0);
    assert!(app.key_modes.on(&sid).is_none(), "y leaves copy mode");
    assert!(!app.backends[&sid].copy_mode());
    assert_eq!(app.info.as_deref(), Some("Selection copied"));
}
