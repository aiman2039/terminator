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
    assert!(editor_owns("command+shift+Left", false));
    // Find belongs to the focused editor, not the terminal search.
    assert!(editor_owns("command+F", false));
    // Find in Files also belongs to the editor. On Linux, stored
    // `command+F` already maps to Ctrl+Shift+F.
    assert!(editor_owns("command+shift+F", false));
    assert!(!editor_owns("command+T", false));
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
fn tab_step_defaults_take_no_chord_another_action_uses() {
    let defaults = terminator_core::Settings::default().keybindings;
    for tab in ["next_tab", "prev_tab"] {
        let chord = parse(&defaults[tab]).unwrap_or_else(|| panic!("{tab}"));
        let mac = cfg!(target_os = "macos");
        assert_eq!(chord.modifiers.shift, mac, "{tab}");
        for (action, value) in &defaults {
            if action != tab {
                assert_ne!(parse(value), Some(chord), "{tab} and {action}");
            }
        }
        // Every JetBrains app-wide chord stays free as well.
        for &(keymap, command, _) in jetbrains_app_chords() {
            assert_ne!(parse(keymap), Some(chord), "{tab} and {command:?}");
        }
    }
    // egui ignores an extra Shift, so the Shift chord is checked first.
    let at = |action: Action| Action::ALL.iter().position(|a| *a == action);
    assert!(at(Action::NextTab) < at(Action::NextPane));
    assert!(at(Action::PrevTab) < at(Action::NextPane));
}

#[test]
fn a_shift_bracket_chord_also_takes_the_curly_bracket_key() {
    let ctx = egui::Context::default();
    let chord = parse("command+shift+]").unwrap();
    let press = |key: Key| egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: chord.modifiers,
        }],
        ..Default::default()
    };
    for key in [Key::CloseBracket, Key::CloseCurlyBracket] {
        let mut fired = None;
        let mut output = ctx.run_ui(press(key), |ui| {
            fired = Some(consume(ui.ctx(), "command+shift+]"));
        });
        output.textures_delta.clear();
        assert_eq!(fired, Some(true), "{key:?}");
    }
    // On Linux stored `command` already holds Shift, so this is macOS only.
    if !cfg!(target_os = "macos") {
        return;
    }
    let mut fired = None;
    let mut output = ctx.run_ui(press(Key::CloseCurlyBracket), |ui| {
        fired = Some(consume(ui.ctx(), "command+]"));
    });
    output.textures_delta.clear();
    assert_eq!(fired, Some(false), "a plain bracket chord ignores `}}`");
}

#[test]
fn fills_missing_default_actions() {
    let mut map = BTreeMap::new();
    fill_defaults(&mut map);
    assert!(map.contains_key("open_palette"));
    assert_eq!(binding(&map, "open_file"), "command+O");
}

/// Git actions start unbound: `JetBrains` gives most of them no chord, its
/// Commit (Cmd+K) and Push (Cmd+Shift+K) chords are taken, and Cmd+9 is its
/// Git tool window, not the log.
const UNBOUND_BY_DEFAULT: &[Action] = &[
    Action::GitCommit,
    Action::GitCommitPush,
    Action::GitRefresh,
    Action::GitShowLog,
    Action::GitCompareWith,
    Action::GitCompareRevision,
    Action::GitShowHistory,
    Action::GitAnnotate,
    Action::GitStageAll,
    Action::GitShowAllChanges,
];

#[test]
fn every_menu_action_has_a_unique_default_chord() {
    let defaults = terminator_core::Settings::default().keybindings;
    for action in Action::ALL
        .iter()
        .filter(|action| !UNBOUND_BY_DEFAULT.contains(action))
        .map(|action| action.id())
    {
        let value = defaults.get(action).unwrap_or_else(|| panic!("{action}"));
        assert!(parse(value).is_some(), "{action}={value}");
    }
    assert_eq!(invalid(&defaults), None);
    // Stored `command` already means Ctrl+Shift on Linux, so an extra
    // `shift` token must not be the only difference between two chords,
    // unless focus picks one of them (see `shared`).
    let mut seen = HashMap::<String, String>::new();
    for (action, value) in &defaults {
        // Next/Previous tab use Cmd+Shift+]/[ only on macOS; Linux has its
        // own Ctrl+PageDown/PageUp defaults (see the tab-step test).
        if cfg!(target_os = "macos") && matches!(action.as_str(), "next_tab" | "prev_tab") {
            continue;
        }
        let id = linux_chord_id(value);
        if let Some(other) = seen.insert(id.clone(), action.clone())
            && !shared::partners(action, &other)
        {
            panic!("{action} and {other} collapse to {id} on Linux");
        }
    }
    assert_eq!(parse("command+alt+Up").unwrap().key, Key::ArrowUp);
    assert_eq!(parse("command+shift+Left").unwrap().key, Key::ArrowLeft);
    let shown = display("command+B");
    assert!(shown.contains('B'), "{shown}");
}

#[test]
fn find_chords_reach_their_own_action_first() {
    let defaults = terminator_core::Settings::default().keybindings;
    // `(action, a terminal has the keyboard)`. On Linux, Find in Files and
    // Find in terminal are both Ctrl+Shift+F, and focus picks one.
    let mut cases = vec![
        (Action::FindInFiles, false),
        (Action::FindInTerminal, true),
        (Action::SearchScrollback, true),
        (Action::SearchScrollback, false),
    ];
    if cfg!(target_os = "macos") {
        // Cmd+Shift+F is its own chord there, so a terminal does not keep it.
        cases.push((Action::FindInFiles, true));
    }
    for (wanted, terminal) in cases {
        let chord = parse(&binding(&defaults, wanted.id())).unwrap();
        let ctx = egui::Context::default();
        let mut first = None;
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
                // The same walk as the global shortcut loop.
                first = Action::ALL.iter().copied().find(|&action| {
                    !yields(&defaults, action, terminal)
                        && consume(ui.ctx(), &binding(&defaults, action.id()))
                });
            },
        );
        output.textures_delta.clear();
        assert_eq!(first, Some(wanted), "{} terminal {terminal}", wanted.id());
    }
    assert_eq!(defaults["find_in_files"], "command+shift+F");
    // A focused editor keeps Cmd/Ctrl+Shift+F: it seeds the search with
    // its selection.
    assert!(editor_owns(&defaults["find_in_files"], false));
}

#[test]
fn find_in_files_shares_ctrl_shift_f_with_find_in_terminal_on_linux() {
    let defaults = terminator_core::Settings::default().keybindings;
    let same = parse(&defaults["find_in_files"]) == parse(&defaults["find_in_terminal"]);
    assert_eq!(same, !cfg!(target_os = "macos"));
    assert_eq!(invalid(&defaults), None);
    // Bound alike (the Linux defaults): a focused terminal finds in the
    // terminal, every other place finds in files.
    let mut alike = defaults.clone();
    alike.insert("find_in_terminal".into(), defaults["find_in_files"].clone());
    assert_eq!(invalid(&alike), None);
    assert!(yields(&alike, Action::FindInFiles, true));
    assert!(!yields(&alike, Action::FindInTerminal, true));
    assert!(yields(&alike, Action::FindInTerminal, false));
    assert!(!yields(&alike, Action::FindInFiles, false));
    // Bound apart (the macOS defaults): both work everywhere.
    let mut apart = defaults.clone();
    apart.insert("find_in_terminal".into(), "command+alt+G".into());
    for focus in [true, false] {
        assert!(!yields(&apart, Action::FindInFiles, focus));
        assert!(!yields(&apart, Action::FindInTerminal, focus));
    }
}

#[test]
fn rename_terminal_and_replace_in_files_share_a_chord_by_focus() {
    let defaults = terminator_core::Settings::default().keybindings;
    assert_eq!(defaults["replace_in_files"], defaults["rename_terminal"]);
    // The shared pair is no conflict; a third action on the chord is.
    assert_eq!(invalid(&defaults), None);
    let mut clash = defaults.clone();
    clash.insert("open_file".into(), defaults["rename_terminal"].clone());
    assert!(invalid(&clash).unwrap().contains("same keys"));
    // A focused terminal keeps the chord; every other place replaces.
    assert!(yields(&defaults, Action::ReplaceInFiles, true));
    assert!(!yields(&defaults, Action::RenameTerminal, true));
    assert!(yields(&defaults, Action::RenameTerminal, false));
    assert!(!yields(&defaults, Action::ReplaceInFiles, false));
    assert!(!yields(&defaults, Action::OpenFile, true));
    // A custom Rename terminal chord ends the sharing: both work anywhere.
    let mut custom = defaults.clone();
    custom.insert("rename_terminal".into(), "command+alt+R".into());
    for focus in [true, false] {
        assert!(!yields(&custom, Action::RenameTerminal, focus));
        assert!(!yields(&custom, Action::ReplaceInFiles, focus));
    }
}

#[test]
fn filling_defaults_never_takes_a_chord_the_user_bound() {
    // A map saved before Replace in Files existed, with Cmd+Shift+R moved
    // from Rename terminal to Clear scrollback.
    let mut map = BTreeMap::from([
        ("rename_terminal".to_owned(), "command+alt+R".to_owned()),
        ("clear_scrollback".to_owned(), "command+shift+R".to_owned()),
    ]);
    fill_defaults(&mut map);
    assert_eq!(map["replace_in_files"], "");
    assert_eq!(map["clear_scrollback"], "command+shift+R");
    assert_eq!(invalid(&map), None);
    // Saved defaults: Replace in Files joins Rename terminal's chord.
    let mut saved = BTreeMap::from([("rename_terminal".to_owned(), "command+shift+R".to_owned())]);
    fill_defaults(&mut saved);
    assert_eq!(saved["replace_in_files"], "command+shift+R");
    assert_eq!(saved, terminator_core::Settings::default().keybindings);
    // Find in terminal saved before Find in Files existed: on Linux both
    // are Ctrl+Shift+F, and focus picks one.
    let mut find = BTreeMap::from([("find_in_terminal".to_owned(), "command+F".to_owned())]);
    fill_defaults(&mut find);
    assert_eq!(find["find_in_files"], "command+shift+F");
    assert_eq!(invalid(&find), None);
    // Find in terminal moved to Cmd+Shift+F keeps it on macOS. On Linux
    // that spelling is its default Ctrl+Shift+F, which focus shares.
    let mut moved = BTreeMap::from([("find_in_terminal".to_owned(), "command+shift+F".to_owned())]);
    fill_defaults(&mut moved);
    let expected = if cfg!(target_os = "macos") {
        ""
    } else {
        "command+shift+F"
    };
    assert_eq!(moved["find_in_files"], expected);
    assert_eq!(invalid(&moved), None);
}

#[test]
fn git_actions_start_unbound() {
    let defaults = terminator_core::Settings::default().keybindings;
    let git: Vec<Action> = Action::ALL
        .iter()
        .copied()
        .filter(|action| action.label().starts_with("Git: "))
        .collect();
    assert_eq!(git, UNBOUND_BY_DEFAULT);
    for action in git {
        assert!(action.id().starts_with("git_"), "{}", action.id());
        assert!(!defaults.contains_key(action.id()), "{}", action.id());
        assert_eq!(binding(&defaults, action.id()), "");
        assert_eq!(pretty(&defaults, action.id()), "");
    }
    // The JetBrains Commit, Push, and Update chords keep their owners.
    assert!(
        editor_owns("command+K", false),
        "Cmd+K is the editor's Hover"
    );
    assert_eq!(binding(&defaults, "clear_scrollback"), "command+shift+K");
    assert_eq!(binding(&defaults, "new_terminal"), "command+T");
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

#[test]
fn jetbrains_keymap_claims_sidebar_chords_only_when_on() {
    let declaration = if cfg!(target_os = "macos") {
        "command+B"
    } else {
        "ctrl+B"
    };
    assert!(!editor_owns(declaration, false));
    assert!(editor_owns(declaration, true));
    assert!(editor_owns("F2", true));
    assert!(editor_owns("shift+F6", true));
    // Go to Implementation beats New browser tab in a focused editor.
    let implementation = if cfg!(target_os = "macos") {
        "command+alt+B"
    } else {
        "ctrl+alt+B"
    };
    assert!(editor_owns(implementation, false));
}

#[test]
fn cmd_e_is_recent_files_and_ide_mode_moves_to_a_free_chord() {
    let defaults = terminator_core::Settings::default().keybindings;
    assert_eq!(binding(&defaults, "recent_files"), "command+E");
    assert_eq!(binding(&defaults, "toggle_ide_mode"), "command+alt+E");
    // No editor key takes either chord, so both work in a focused editor.
    for chord in ["command+E", "command+alt+E"] {
        assert!(!editor_owns(chord, true), "{chord}");
    }
    let ide = parse("command+alt+E");
    assert!(
        jetbrains_app_chords()
            .iter()
            .all(|(chord, _, _)| parse(chord) != ide)
    );
}

fn keymap_reach(command: JetBrainsCommand) -> (bool, bool, bool) {
    let (_, _, reach) = jetbrains_app_chords()
        .iter()
        .find(|(_, candidate, _)| *candidate == command)
        .unwrap_or_else(|| panic!("{command:?}"));
    // No focus (Explorer, Git, Settings), a focused editor, a focused terminal.
    (
        reach.applies(false, false),
        reach.applies(true, false),
        reach.applies(false, true),
    )
}

#[test]
fn keymap_palette_chords_reach_every_pane_and_history_stays_in_the_editor() {
    for command in [JetBrainsCommand::FindAction, JetBrainsCommand::GotoFile] {
        let (anywhere, editor, _) = keymap_reach(command);
        assert!(anywhere && editor, "{command:?}");
    }
    assert!(keymap_reach(JetBrainsCommand::GotoFile).2);
    if cfg!(target_os = "macos") {
        assert!(keymap_reach(JetBrainsCommand::FindAction).2);
        for action in [Action::NavigateBack, Action::NavigateForward] {
            // Cmd+] is Next pane outside the editor.
            let reach = keymap_reach(JetBrainsCommand::Run(action));
            assert_eq!(reach, (false, true, false), "{action:?}");
        }
    } else {
        // A focused terminal keeps Ctrl+E (line end) and Ctrl+Shift+A
        // (Select all); every other pane gets the keymap chord.
        for command in [
            JetBrainsCommand::FindAction,
            JetBrainsCommand::Run(Action::RecentFiles),
        ] {
            assert_eq!(keymap_reach(command), (true, true, false), "{command:?}");
        }
    }
}

#[test]
fn keymap_chords_for_every_pane_shadow_no_default_binding() {
    let defaults = terminator_core::Settings::default().keybindings;
    for &(chord, command, reach) in jetbrains_app_chords() {
        let parsed = parse(chord).unwrap_or_else(|| panic!("{chord}"));
        if !reach.applies(false, true) {
            continue;
        }
        let shadowed: Vec<_> = defaults
            .iter()
            .filter(|(_, value)| parse(value) == Some(parsed))
            .map(|(action, _)| action.as_str())
            .collect();
        assert!(
            shadowed.is_empty(),
            "{chord} ({command:?}) hides {shadowed:?}"
        );
    }
}

fn old_default_bindings() -> BTreeMap<String, String> {
    let mut map = terminator_core::Settings::default().keybindings;
    map.insert("toggle_ide_mode".into(), "command+E".into());
    map.insert("recent_files".into(), "command+alt+E".into());
    map
}

#[test]
fn old_default_bindings_move_to_the_new_defaults_once() {
    let mut map = old_default_bindings();
    assert!(migrate_moved_defaults(&mut map));
    assert_eq!(map, terminator_core::Settings::default().keybindings);
    assert!(!migrate_moved_defaults(&mut map));
    let mut missing = BTreeMap::new();
    assert!(!migrate_moved_defaults(&mut missing));
    assert!(missing.is_empty());
}

#[test]
fn customized_bindings_stay_while_old_defaults_move() {
    let mut map = old_default_bindings();
    map.insert("toggle_ide_mode".into(), "command+I".into());
    assert!(migrate_moved_defaults(&mut map));
    assert_eq!(binding(&map, "toggle_ide_mode"), "command+I");
    assert_eq!(binding(&map, "recent_files"), "command+E");
    // A changed or unbound Recent files stays; Toggle IDE mode still moves.
    for custom in ["command+R", ""] {
        let mut map = old_default_bindings();
        map.insert("recent_files".into(), custom.into());
        assert!(migrate_moved_defaults(&mut map));
        assert_eq!(binding(&map, "toggle_ide_mode"), "command+alt+E");
        assert_eq!(binding(&map, "recent_files"), custom);
    }
    let mut map = old_default_bindings();
    map.insert("toggle_ide_mode".into(), "command+I".into());
    map.insert("recent_files".into(), "command+R".into());
    let before = map.clone();
    assert!(!migrate_moved_defaults(&mut map));
    assert_eq!(map, before);
}

#[test]
fn a_moved_default_never_takes_a_chord_another_action_uses() {
    // The user put Cmd+E on another action: Recent files stays.
    let mut map = old_default_bindings();
    map.insert("toggle_ide_mode".into(), "command+I".into());
    map.insert("new_browser".into(), "command+E".into());
    let before = map.clone();
    assert!(!migrate_moved_defaults(&mut map));
    assert_eq!(map, before);
    // Recent files kept the new IDE-mode chord in another spelling.
    let mut map = old_default_bindings();
    map.insert("recent_files".into(), "alt+command+E".into());
    let before = map.clone();
    assert!(!migrate_moved_defaults(&mut map));
    assert_eq!(map, before);
}

#[test]
fn terminal_focus_needs_a_focused_terminal_painted_last_pass() {
    let ctx = egui::Context::default();
    let mut ids = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let terminal = ui.allocate_response(egui::vec2(40.0, 20.0), egui::Sense::click());
        let field = ui.allocate_response(egui::vec2(40.0, 20.0), egui::Sense::click());
        mark_terminal(ui.ctx(), terminal.id);
        terminal.request_focus();
        ids = Some((terminal.id, field.id));
    });
    output.textures_delta.clear();
    let (terminal, field) = ids.unwrap();
    assert!(terminal_has_focus(&ctx));
    ctx.memory_mut(|memory| memory.request_focus(field));
    assert!(!terminal_has_focus(&ctx));
    // A terminal that stopped painting no longer counts.
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
    }
    ctx.memory_mut(|memory| memory.request_focus(terminal));
    assert!(!terminal_has_focus(&ctx));
}

#[test]
fn double_shift_fires_on_two_clean_taps_only() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut taps = DoubleShift::default();
    assert!(!taps.update(true, false, at(0)));
    assert!(!taps.update(false, false, at(50)));
    assert!(!taps.update(true, false, at(150)));
    assert!(taps.update(false, false, at(200)));
    // Shift held while typing a capital letter is not a tap.
    let mut typing = DoubleShift::default();
    typing.update(true, false, at(0));
    typing.update(false, false, at(50));
    typing.update(true, false, at(100));
    typing.update(true, true, at(120));
    assert!(!typing.update(false, false, at(150)));
    // Taps too far apart do not count.
    let mut slow = DoubleShift::default();
    slow.update(true, false, at(0));
    slow.update(false, false, at(10));
    slow.update(true, false, at(900));
    assert!(!slow.update(false, false, at(910)));
}

const TOOL_WINDOW_KEYS: [&str; 6] = [
    "tool_explorer",
    "tool_problems",
    "tool_structure",
    "tool_git",
    "tool_terminal",
    "select_in",
];

#[test]
fn tool_window_keys_follow_the_jetbrains_keymap_on_each_platform() {
    let defaults = terminator_core::Settings::default().keybindings;
    let mac = cfg!(target_os = "macos");
    for (action, digit) in [
        ("tool_explorer", 1),
        ("tool_problems", 6),
        ("tool_structure", 7),
        ("tool_git", 9),
    ] {
        let expected = if mac {
            format!("command+{digit}")
        } else {
            format!("alt+{digit}")
        };
        assert_eq!(binding(&defaults, action), expected);
    }
    assert_eq!(binding(&defaults, "tool_terminal"), "alt+F12");
    assert_eq!(binding(&defaults, "select_in"), "alt+F1");
    if mac {
        assert_eq!(display("command+1"), "⌘1");
        assert_eq!(display("alt+F12"), "⌥F12");
    } else {
        assert_eq!(display("alt+1"), "Alt+1");
        assert_eq!(display("alt+F1"), "Alt+F1");
    }
    // No editor key and no keymap chord takes them: they work in an editor.
    let keymap: Vec<_> = jetbrains_app_chords()
        .iter()
        .filter_map(|(chord, _, _)| parse(chord))
        .collect();
    for action in TOOL_WINDOW_KEYS {
        let value = binding(&defaults, action);
        assert!(!editor_owns(&value, true), "{value}");
        assert!(!editor_owns(&value, false), "{value}");
        assert!(!keymap.contains(&parse(&value).unwrap()), "{value}");
    }
}

#[test]
fn a_focused_linux_terminal_keeps_alt_letters_and_digits() {
    let linux = cfg!(not(target_os = "macos"));
    assert_eq!(terminal_keeps("alt+1"), linux);
    assert_eq!(terminal_keeps("alt+X"), linux);
    for chord in [
        "alt+F12",
        "alt+F1",
        "command+1",
        "command+T",
        "F7",
        "alt+shift+F7",
    ] {
        assert!(!terminal_keeps(chord), "{chord}");
    }
}

fn texts(input: &egui::InputState) -> Vec<String> {
    input
        .events
        .iter()
        .filter_map(|event| match event {
            egui::Event::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn consuming_an_alt_digit_drops_the_character_it_types() {
    let ctx = egui::Context::default();
    let alt_one = egui::Event::Key {
        key: Key::Num1,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::ALT,
    };
    let text = |text: &str| egui::Event::Text(text.into());
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![text("a"), alt_one, text("1"), text("b")],
            ..Default::default()
        },
        |ui| {
            assert!(consume(ui.ctx(), "alt+1"));
            ui.input(|input| {
                assert_eq!(texts(input), ["a", "b"]);
                assert!(
                    !input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Key { .. }))
                );
            });
        },
    );
    output.textures_delta.clear();
}
