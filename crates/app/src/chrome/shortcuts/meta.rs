//! Alt (Meta) chords that also type a character, such as the Linux tool
//! window keys Alt+1 and Alt+9, and chords pressed with an extra modifier.
use super::{Chord, parse};
use eframe::egui;

/// An Alt chord without Ctrl or Command whose key types a character. On
/// Linux winit sends that character as a text event after the key.
pub(super) fn types_text(chord: Chord) -> bool {
    let modifiers = chord.modifiers;
    if !modifiers.alt || modifiers.ctrl || modifiers.command || modifiers.mac_cmd {
        return false;
    }
    let mut symbol = chord.key.symbol_or_name().chars();
    matches!((symbol.next(), symbol.next()), (Some(c), None) if c.is_ascii_graphic())
}

/// Consume the chord's key presses and the text event right after each
/// one, so the character is not typed into the focused field or editor.
pub(super) fn consume_with_text(input: &mut egui::InputState, chord: Chord) -> bool {
    let mut found = false;
    let mut after_key = false;
    input.events.retain(|event| {
        let hit = matches!(
            event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == chord.key && modifiers.matches_logically(chord.modifiers)
        );
        let typed = after_key && matches!(event, egui::Event::Text(_));
        after_key = hit;
        found |= hit;
        !(hit || typed)
    });
    found
}

/// Shift+[ and Shift+] can arrive as `{` and `}` (the logical key of the
/// layout); a stored `shift+[` chord takes those too.
pub(super) fn shifted_key(chord: Chord) -> Option<egui::Key> {
    if !chord.modifiers.shift {
        return None;
    }
    match chord.key {
        egui::Key::OpenBracket => Some(egui::Key::OpenCurlyBracket),
        egui::Key::CloseBracket => Some(egui::Key::CloseCurlyBracket),
        _ => None,
    }
}

/// The chord's key is pressed with an Alt or Shift the chord lacks. egui
/// matches ignoring an extra Alt or Shift, but each makes its own shortcut
/// (`Cmd+Alt+L` reformats, not `Cmd+L`; `Shift+F7` is not `F7`). Shift
/// counts only on keys whose name it does not change: a symbol key may
/// need Shift to type at all.
pub(super) fn modifier_variant(input: &egui::InputState, chord: Chord) -> bool {
    let alt = !chord.modifiers.alt;
    let shift = !chord.modifiers.shift && shift_keeps_key(chord.key);
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == chord.key && ((alt && modifiers.alt) || (shift && modifiers.shift))
        )
    })
}

/// Letters, function keys and editing or navigation keys: Shift with one
/// of these is a different chord, never needed to type the key.
fn shift_keeps_key(key: egui::Key) -> bool {
    use egui::Key;
    let name = key.name();
    let letter = name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic());
    let function = name
        .strip_prefix('F')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
    letter
        || function
        || matches!(
            key,
            Key::ArrowDown
                | Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::Escape
                | Key::Tab
                | Key::Backspace
                | Key::Enter
                | Key::Space
                | Key::Insert
                | Key::Delete
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
        )
}

/// A focused terminal keeps this chord. On Linux, Alt+letter and
/// Alt+digit are Meta keys for the shell, Emacs, tmux, and IRC clients.
pub fn terminal_keeps(value: &str) -> bool {
    cfg!(not(target_os = "macos")) && parse(value).is_some_and(types_text)
}

/// The saved token of a chord's key: punctuation as its character.
pub(super) fn key_token(key: egui::Key) -> String {
    use egui::Key;
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
