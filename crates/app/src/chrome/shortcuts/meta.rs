//! Alt (Meta) chords that also type a character, such as the Linux tool
//! window keys Alt+1 and Alt+9.
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

/// A focused terminal keeps this chord. On Linux, Alt+letter and
/// Alt+digit are Meta keys for the shell, Emacs, tmux, and IRC clients.
pub fn terminal_keeps(value: &str) -> bool {
    cfg!(not(target_os = "macos")) && parse(value).is_some_and(types_text)
}
