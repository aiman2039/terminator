//! Parse, display, and consume Settings keybinding strings.
#[cfg(test)]
mod tests;

use eframe::egui::{self, Key, Modifiers};
use std::collections::{BTreeMap, HashMap};

macro_rules! actions {
    ($($variant:ident => $id:literal, $label:literal, $icon:literal;)*) => {
        /// A shortcut or palette command. `id` is the persisted
        /// keybinding key; Settings stay string-keyed. `label` and `icon`
        /// are the shared presentation for menus, toolbars and the palette.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action {
            $($variant,)*
        }

        impl Action {
            /// Every action, in Settings and dispatch order.
            pub const ALL: &[Action] = &[$(Action::$variant,)*];

            pub const fn id(self) -> &'static str {
                match self {
                    $(Action::$variant => $id,)*
                }
            }

            pub const fn label(self) -> &'static str {
                match self {
                    $(Action::$variant => $label,)*
                }
            }

            /// Lucide icon name from the shared UI icon table.
            pub const fn icon(self) -> &'static str {
                match self {
                    $(Action::$variant => $icon,)*
                }
            }
        }
    };
}

actions! {
    NewTerminal => "new_terminal", "New terminal", "Terminal";
    OpenFile => "open_file", "Open file", "File";
    SplitUp => "split_up", "Split up", "PanelTopClose";
    SplitDown => "split_down", "Split down", "PanelBottomClose";
    SplitLeft => "split_left", "Split left", "PanelLeftClose";
    SplitRight => "split_right", "Split right", "PanelRightClose";
    NextPane => "next_pane", "Next pane", "ArrowRight";
    SelectAll => "select_all", "Select all", "TextSelect";
    FindInTerminal => "find_in_terminal", "Find in terminal", "Search";
    SearchScrollback => "search_scrollback", "Search scrollback", "Search";
    CopyWorkingDirectory => "copy_working_directory", "Copy working directory", "Folder";
    RenameTerminal => "rename_terminal", "Rename terminal", "Pencil";
    CloseSession => "close_session", "Close session", "X";
    ClearScrollback => "clear_scrollback", "Clear saved scrollback", "Eraser";
    EditorSave => "editor_save", "Save all", "Save";
    CompareDisk => "compare_disk", "Compare disk", "FileDiff";
    ToggleLeftSidebar => "toggle_left_sidebar", "Toggle left sidebar", "PanelLeft";
    ToggleRightSidebar => "toggle_right_sidebar", "Toggle right sidebar", "PanelRight";
    ToggleIdeMode => "toggle_ide_mode", "Toggle IDE mode", "LayoutDashboard";
    MoveToMain => "move_to_main", "Move terminal to main pane", "ArrowUp";
    MoveToStrip => "move_to_strip", "Move terminal to lower pane", "ArrowDown";
    OpenSettings => "open_settings", "Settings", "Settings";
    OpenPalette => "open_palette", "Search Everywhere", "Search";
    GotoSymbol => "goto_symbol", "Go to symbol", "ListTree";
    FileStructure => "file_structure", "File structure", "ListTree";
    NavigateBack => "navigate_back", "Back", "ArrowLeft";
    NavigateForward => "navigate_forward", "Forward", "ArrowRight";
    RecentFiles => "recent_files", "Recent files", "History";
    NextAttention => "next_attention", "Next agent needing attention", "Bell";
    NewBrowser => "new_browser", "New browser tab", "Globe";
    NewVnc => "new_vnc", "New VNC viewer", "Monitor";
    DiffNextChange => "diff_next_change", "Diff: next change", "ArrowDown";
    DiffPrevChange => "diff_prev_change", "Diff: previous change", "ArrowUp";
    DiffNextFile => "diff_next_file", "Diff: next file", "ArrowRight";
    DiffPrevFile => "diff_prev_file", "Diff: previous file", "ArrowLeft";
    // Backward first: egui ignores extra Shift, so Ctrl+Tab would also
    // consume Ctrl+Shift+Tab.
    SwitcherBack => "tab_switcher_back", "Recent tab switcher (backward)", "Files";
    SwitcherNext => "tab_switcher", "Recent tab switcher", "Files";
}

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
    let modifier_parts = parts.get(..parts.len().saturating_sub(1))?;
    for part in modifier_parts {
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
    // egui matches ignoring extra Alt; an Alt chord is its own shortcut
    // (`Cmd+Alt+L` reformats, it does not toggle the sidebar on `Cmd+L`).
    let alt_variant = !chord.modifiers.alt
        && ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { key, pressed: true, modifiers, .. }
                        if *key == chord.key && modifiers.alt
                )
            })
        });
    if alt_variant {
        return false;
    }
    ctx.input_mut(|input| input.consume_key(chord.modifiers, chord.key))
}

/// True when a focused native editor owns this chord as an IDE caret
/// motion or line edit (Cmd+Shift+Left selects to line start there, it
/// does not split the pane), or project navigation. The editor wins,
/// as in `IntelliJ`.
pub fn editor_owns(value: &str) -> bool {
    parse(value).is_some_and(|chord| {
        terminator_native_edit::view::ide_edit_key(terminator_native_edit::view::IdeEditKeyInput {
            key: chord.key,
            modifiers: chord.modifiers,
            mac: cfg!(target_os = "macos"),
            page_rows: 1,
        })
        .is_some()
            || terminator_native_edit::view::ide_shortcut(chord.key, chord.modifiers).is_some()
            || terminator_native_edit::view::ide_navigation_shortcut(
                chord.key,
                chord.modifiers,
                false,
            )
            .is_some()
    })
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
                let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                out.push((left.clone(), right.clone()));
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
