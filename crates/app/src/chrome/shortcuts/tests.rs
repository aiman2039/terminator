use super::*;

#[test]
fn explicit_empty_binding_stays_disabled() {
    let mut map = BTreeMap::new();
    assert_eq!(binding(&map, "new_terminal"), "command+T");
    map.insert("new_terminal".into(), String::new());
    fill_defaults(&mut map);
    assert_eq!(binding(&map, "new_terminal"), "");
    assert_eq!(pretty(&map, "new_terminal"), "");
    assert!(!consume(
        &egui::Context::default(),
        &binding(&map, "new_terminal")
    ));
}

#[test]
fn action_ids_are_unique() {
    let ids: std::collections::HashSet<_> = Action::ALL.iter().map(|a| a.id()).collect();
    assert_eq!(ids.len(), Action::ALL.len());
}

#[test]
fn parses_default_chords() {
    assert!(parse("command+T").is_some());
    assert!(parse("command+shift+D").is_some());
    assert!(parse("command+alt+D").is_some());
    assert!(parse("command+]").is_some());
    assert!(parse("command+,").is_some());
    assert!(parse("command+P").is_some());
    assert!(parse("not-a-key").is_none());
    assert!(parse("command+f1x").is_none());
    // Text navigation chords belong to a focused editor.
    assert!(editor_owns("command+shift+Left"));
    // Find belongs to the focused editor, not the terminal search.
    assert!(editor_owns("command+F"));
    // Find in Files also belongs to the editor. On Linux, stored
    // `command+F` already maps to Ctrl+Shift+F.
    assert!(editor_owns("command+shift+F"));
    assert!(!editor_owns("command+T"));
}

#[test]
fn capture_round_trips_letter_chords() {
    let parsed = parse("command+T").unwrap();
    let stored = from_input(parsed.modifiers, parsed.key).unwrap();
    assert_eq!(parse(&stored), Some(parsed));
    assert!(!display("command+T").is_empty());
}

#[test]
fn reports_conflicts_and_unknown_tokens() {
    let mut map = BTreeMap::new();
    map.insert("new_terminal".into(), "command+T".into());
    map.insert("open_file".into(), "command+T".into());
    assert!(invalid(&map).unwrap().contains("same keys"));
    map.insert("open_file".into(), "nope".into());
    assert!(invalid(&map).unwrap().contains("Unknown shortcut"));
    map.insert("open_file".into(), "command+O".into());
    assert_eq!(invalid(&map), None);
}

#[test]
fn alt_variants_do_not_trigger_the_plain_shortcut() {
    let ctx = egui::Context::default();
    let press = |modifiers: Modifiers| egui::RawInput {
        events: vec![egui::Event::Key {
            key: Key::L,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    let plain = parse("command+L").unwrap().modifiers;
    let alt = Modifiers { alt: true, ..plain };
    let mut fired = None;
    let mut output = ctx.run_ui(press(alt), |ui| {
        fired = Some(consume(ui.ctx(), "command+L"));
    });
    output.textures_delta.clear();
    assert_eq!(fired, Some(false));
    let mut output = ctx.run_ui(press(plain), |ui| {
        fired = Some(consume(ui.ctx(), "command+L"));
    });
    output.textures_delta.clear();
    assert_eq!(fired, Some(true));
}

#[test]
fn fills_missing_default_actions() {
    let mut map = BTreeMap::new();
    fill_defaults(&mut map);
    assert!(map.contains_key("open_palette"));
    assert_eq!(binding(&map, "open_file"), "command+O");
}

#[test]
fn every_menu_action_has_a_unique_default_chord() {
    let defaults = terminator_core::Settings::default().keybindings;
    for action in Action::ALL.iter().map(|action| action.id()) {
        let value = defaults.get(action).unwrap_or_else(|| panic!("{action}"));
        assert!(parse(value).is_some(), "{action}={value}");
    }
    assert_eq!(invalid(&defaults), None);
    // Stored `command` already means Ctrl+Shift on Linux, so an extra
    // `shift` token must not be the only difference between two chords.
    let mut seen = HashMap::<String, String>::new();
    for (action, value) in &defaults {
        let id = linux_chord_id(value);
        if let Some(other) = seen.insert(id.clone(), action.clone()) {
            panic!("{action} and {other} collapse to {id} on Linux");
        }
    }
    assert_eq!(parse("command+alt+Up").unwrap().key, Key::ArrowUp);
    assert_eq!(parse("command+shift+Left").unwrap().key, Key::ArrowLeft);
    let shown = display("command+B");
    assert!(shown.contains('B'), "{shown}");
}

fn linux_chord_id(value: &str) -> String {
    let mut parts = value
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key = parts.pop().unwrap_or_default();
    if parts.iter().any(|part| part == "command") && !parts.iter().any(|part| part == "shift") {
        parts.push("shift".into());
    }
    parts.sort();
    parts.dedup();
    format!("{}+{key}", parts.join("+"))
}

#[test]
fn captured_function_and_navigation_keys_round_trip() {
    for key in [
        Key::F1,
        Key::F12,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Delete,
        Key::Insert,
        Key::ArrowLeft,
        Key::ArrowRight,
    ] {
        let modifiers = parse("command+T").unwrap().modifiers;
        let stored = from_input(modifiers, key).unwrap();
        assert_eq!(parse(&stored), Some(Chord { modifiers, key }), "{stored}");
        let map = BTreeMap::from([("new_terminal".into(), stored)]);
        assert_eq!(invalid(&map), None);
    }
}

#[test]
fn consuming_a_registered_shortcut_removes_the_key_event() {
    let chord = parse("command+T").unwrap();
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: chord.key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: chord.modifiers,
            }],
            ..Default::default()
        },
        |ui| {
            assert!(consume(ui.ctx(), "command+T"));
            ui.ctx().input(|input| {
                assert!(
                    !input
                        .events
                        .iter()
                        .any(|event| { matches!(event, egui::Event::Key { key: Key::T, .. }) })
                );
            });
        },
    );
    output.textures_delta.clear();
}

#[test]
#[cfg(target_os = "macos")]
fn captured_control_shortcut_consumes_control_without_command() {
    let value = from_input(Modifiers::CTRL, Key::T).unwrap();
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::T,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL,
            }],
            ..Default::default()
        },
        |ui| assert!(consume(ui.ctx(), &value)),
    );
    output.textures_delta.clear();
    assert_eq!(parse("command+ctrl+T"), parse("ctrl+command+T"));
    assert!(parse("command+ctrl+T").unwrap().modifiers.command);
}
