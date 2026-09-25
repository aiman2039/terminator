//! Parse, display, and consume Settings keybinding strings.
use eframe::egui::{self, Key, Modifiers};
use std::collections::{BTreeMap, HashMap};

pub const ACTIONS: &[(&str, &str)] = &[
    ("new_terminal", "New terminal"),
    ("open_file", "Open file"),
    ("split_up", "Split up"),
    ("split_down", "Split down"),
    ("split_left", "Split left"),
    ("split_right", "Split right"),
    ("next_pane", "Next pane"),
    ("select_all", "Select all"),
    ("find_in_terminal", "Find in terminal"),
    ("search_scrollback", "Search scrollback"),
    ("copy_working_directory", "Copy working directory"),
    ("rename_terminal", "Rename terminal"),
    ("close_session", "Close session"),
    ("clear_scrollback", "Clear saved scrollback"),
    ("editor_save", "Save all"),
    ("compare_disk", "Compare disk"),
    ("toggle_left_sidebar", "Toggle left sidebar"),
    ("toggle_right_sidebar", "Toggle right sidebar"),
    ("toggle_ide_mode", "Toggle IDE mode"),
    ("open_settings", "Settings"),
    ("open_palette", "Command palette"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

pub fn parse(value: &str) -> Option<Chord> {
    let parts = value
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key_token = parts.last()?;
    let key = parse_key(key_token)?;
    let mut modifiers = Modifiers::NONE;
    for part in &parts[..parts.len().saturating_sub(1)] {
        match part.as_str() {
            "command" | "cmd" | "super" | "meta" => {
                if cfg!(target_os = "macos") {
                    modifiers.mac_cmd = true;
                    modifiers.command = true;
                } else {
                    // Stored `command` means Ctrl+Shift on Linux, matching existing help text.
                    modifiers.ctrl = true;
                    modifiers.command = true;
                    if *part == "command" {
                        modifiers.shift = true;
                    }
                }
            }
            "ctrl" | "control" => {
                modifiers.ctrl = true;
                if !cfg!(target_os = "macos") {
                    modifiers.command = true;
                }
            }
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    Some(Chord { modifiers, key })
}

fn parse_key(token: &str) -> Option<Key> {
    Key::from_name(token)
        .or_else(|| {
            Key::ALL
                .iter()
                .copied()
                .find(|key| key.name().eq_ignore_ascii_case(token))
        })
        .or(match token {
            "," | "comma" => Some(Key::Comma),
            "." | "period" | "dot" => Some(Key::Period),
            ";" | "semicolon" => Some(Key::Semicolon),
            "'" | "quote" => Some(Key::Quote),
            "/" | "slash" => Some(Key::Slash),
            "\\" | "backslash" => Some(Key::Backslash),
            "[" => Some(Key::OpenBracket),
            "]" => Some(Key::CloseBracket),
            "-" | "minus" => Some(Key::Minus),
            "=" | "equals" => Some(Key::Equals),
            "`" | "backtick" => Some(Key::Backtick),
            "space" => Some(Key::Space),
            "enter" | "return" => Some(Key::Enter),
            "esc" | "escape" => Some(Key::Escape),
            "tab" => Some(Key::Tab),
            "backspace" => Some(Key::Backspace),
            _ => None,
        })
}

pub fn display(value: &str) -> String {
    let Some(chord) = parse(value) else {
        return value.to_string();
    };
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if chord.modifiers.mac_cmd || chord.modifiers.command && !chord.modifiers.ctrl {
            parts.push("⌘");
        }
        if chord.modifiers.ctrl && !chord.modifiers.mac_cmd {
            parts.push("⌃");
        }
        if chord.modifiers.alt {
            parts.push("⌥");
        }
        if chord.modifiers.shift {
            parts.push("⇧");
        }
        format!("{}{}", parts.join(""), key_label(chord.key))
    } else {
        if chord.modifiers.ctrl || chord.modifiers.command {
            parts.push("Ctrl");
        }
        if chord.modifiers.alt {
            parts.push("Alt");
        }
        if chord.modifiers.shift {
            parts.push("Shift");
        }
        parts.push(key_label(chord.key));
        parts.join("+")
    }
}

fn key_label(key: Key) -> &'static str {
    match key {
        Key::Comma => ",",
        Key::Period => ".",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Backtick => "`",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        other => other.symbol_or_name(),
    }
}

pub fn from_input(modifiers: Modifiers, key: Key) -> Option<String> {
    if matches!(
        key,
        Key::Escape
            | Key::Tab
            | Key::Enter
            | Key::Backspace
            | Key::Delete
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
    ) && !modifiers.any()
    {
        return None;
    }
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if modifiers.mac_cmd || modifiers.command {
            parts.push("command");
        }
        if modifiers.ctrl && !modifiers.mac_cmd {
            parts.push("ctrl");
        }
    } else if modifiers.ctrl && modifiers.shift && !modifiers.alt {
        parts.push("command");
    } else if modifiers.ctrl || modifiers.command {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift
        && !(cfg!(not(target_os = "macos")) && parts.first().is_some_and(|p| *p == "command"))
    {
        parts.push("shift");
    }
    if parts.is_empty() {
        return None;
    }
    let key = key_token(key);
    parts.push(&key);
    Some(parts.join("+"))
}

fn key_token(key: Key) -> String {
    match key {
        Key::Comma => ",".into(),
        Key::Period => ".".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Minus => "-".into(),
        Key::Equals => "=".into(),
        Key::Slash => "/".into(),
        Key::Backslash => "\\".into(),
        Key::Semicolon => ";".into(),
        Key::Quote => "'".into(),
        Key::Backtick => "`".into(),
        Key::Space => "space".into(),
        other => other.symbol_or_name().to_string(),
    }
}

pub fn consume(ctx: &egui::Context, value: &str) -> bool {
    let Some(chord) = parse(value) else {
        return false;
    };
    ctx.input_mut(|input| input.consume_key(chord.modifiers, chord.key))
}

pub fn binding(map: &BTreeMap<String, String>, action: &str) -> String {
    map.get(action)
        .cloned()
        .or_else(|| {
            terminator_core::Settings::default()
                .keybindings
                .get(action)
                .cloned()
        })
        .unwrap_or_default()
}

pub fn pretty(map: &BTreeMap<String, String>, action: &str) -> String {
    let value = binding(map, action);
    if value.is_empty() {
        String::new()
    } else {
        display(&value)
    }
}

pub fn conflicts(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut by_chord: HashMap<String, Vec<String>> = HashMap::new();
    for (action, value) in map {
        if value.trim().is_empty() {
            continue;
        }
        let Some(chord) = parse(value) else {
            continue;
        };
        by_chord
            .entry(format!("{chord:?}"))
            .or_default()
            .push(action.clone());
    }
    let mut out = Vec::new();
    for actions in by_chord.into_values() {
        if actions.len() > 1 {
            for pair in actions.windows(2) {
                out.push((pair[0].clone(), pair[1].clone()));
            }
        }
    }
    out
}

pub fn invalid(map: &BTreeMap<String, String>) -> Option<String> {
    for (action, value) in map {
        if value.trim().is_empty() {
            continue;
        }
        if parse(value).is_none() {
            return Some(format!("Unknown shortcut for {action}: {value}"));
        }
    }
    let clashes = conflicts(map);
    if let Some((left, right)) = clashes.first() {
        return Some(format!(
            "Shortcuts for {left} and {right} use the same keys"
        ));
    }
    None
}

pub fn fill_defaults(map: &mut BTreeMap<String, String>) {
    for (action, value) in terminator_core::Settings::default().keybindings {
        map.entry(action).or_insert(value);
    }
}

#[cfg(test)]
mod tests {
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
    fn parses_default_chords() {
        assert!(parse("command+T").is_some());
        assert!(parse("command+shift+D").is_some());
        assert!(parse("command+alt+D").is_some());
        assert!(parse("command+]").is_some());
        assert!(parse("command+,").is_some());
        assert!(parse("command+P").is_some());
        assert!(parse("not-a-key").is_none());
        assert!(parse("command+f1x").is_none());
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
    fn fills_missing_default_actions() {
        let mut map = BTreeMap::new();
        fill_defaults(&mut map);
        assert!(map.contains_key("open_palette"));
        assert_eq!(binding(&map, "open_file"), "command+O");
    }

    #[test]
    fn every_menu_action_has_a_unique_default_chord() {
        let defaults = terminator_core::Settings::default().keybindings;
        for (action, _) in ACTIONS {
            let value = defaults.get(*action).unwrap_or_else(|| panic!("{action}"));
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
        let mut output =
            ctx.run_ui(
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
                        assert!(!input.events.iter().any(|event| {
                            matches!(event, egui::Event::Key { key: Key::T, .. })
                        }));
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
}
