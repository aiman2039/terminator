use super::super::vim::{Key as VKey, ModalEngine, Mode};
use egui;
use egui::Color32;
pub fn egui_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    if modifiers.command || modifiers.ctrl {
        return None;
    }
    Some(match key {
        egui::Key::ArrowLeft => VKey::Left,
        egui::Key::ArrowRight => VKey::Right,
        egui::Key::ArrowUp => VKey::Up,
        egui::Key::ArrowDown => VKey::Down,
        egui::Key::Home => VKey::Home,
        egui::Key::End => VKey::End,
        egui::Key::Enter => VKey::Enter,
        egui::Key::Backspace => VKey::Backspace,
        egui::Key::Delete => VKey::Delete,
        egui::Key::Escape => VKey::Escape,
        egui::Key::Tab => VKey::Tab,
        // Punctuation arrives as dedicated physical keys; pass the character
        // through so current and future engine commands stay reachable.
        egui::Key::Colon => VKey::Char(':'),
        egui::Key::Semicolon => VKey::Char(';'),
        egui::Key::Slash => VKey::Char('/'),
        egui::Key::Period => VKey::Char('.'),
        egui::Key::Comma => VKey::Char(','),
        egui::Key::Minus => VKey::Char('-'),
        egui::Key::Equals => VKey::Char('='),
        egui::Key::Space => VKey::Char(' '),
        egui::Key::Quote => VKey::Char('\''),
        egui::Key::OpenBracket => VKey::Char('['),
        egui::Key::CloseBracket => VKey::Char(']'),
        egui::Key::Backslash => VKey::Char('\\'),
        egui::Key::Backtick => VKey::Char('`'),
        _ => return key_name_char(key, modifiers),
    })
}

fn key_name_char(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(VKey::Char(if modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        })),
        // Shifted digits are their US symbols (`1`→`!`, …, `0`→`)`); a bare
        // digit is a count. Positional like the rest of this table.
        (Some(c), None) if c.is_ascii_digit() => Some(VKey::Char(if modifiers.shift {
            const SYMBOLS: &[u8; 10] = b")!@#$%^&*(";
            let index = (c as u8).checked_sub(b'0').map(usize::from).unwrap_or(0);
            char::from(SYMBOLS.get(index).copied().unwrap_or(b')'))
        } else {
            c
        })),
        _ => {
            // Backends that report the symbol itself (`!` instead of shifted
            // `1`) expose it via `symbol_or_name`; pass printable punctuation
            // through so `:q!` stays typeable everywhere.
            let symbol = key.symbol_or_name();
            let mut chars = symbol.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_punctuation() => Some(VKey::Char(c)),
                _ => None,
            }
        }
    }
}

/// IntelliJ-style shortcut the file view owns in every mode (vim included):
/// the editor keeps copy/cut/paste/select/undo keys instead of yielding them.
/// Ctrl-V and Ctrl-H are conditional (see [`ctrl_v_is_block`] and
/// [`ctrl_h_is_motion`]): vim owns them in its modes, the IDE elsewhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdeAction {
    Copy,
    Cut,
    SelectAll,
    Undo,
    Redo,
    /// Consume only; the pasted text arrives as a separate Paste event.
    PasteKey,
    Save,
    Find,
    Replace,
    /// Language-server hover for the cursor (Ctrl+K; vim `K`).
    Hover,
}

/// Resolve a Command/Ctrl-held key to an IDE action. Plain keys and the Vim
/// engine's own keys return `None` and keep their existing routing.
/// (`command` mirrors Ctrl on Linux/Windows, so only Alt is excluded.)
#[must_use]
pub fn ide_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<IdeAction> {
    if !(modifiers.command && !modifiers.alt) {
        return None;
    }
    Some(match key {
        egui::Key::C => IdeAction::Copy,
        egui::Key::X => IdeAction::Cut,
        egui::Key::A => IdeAction::SelectAll,
        egui::Key::Z if modifiers.shift => IdeAction::Redo,
        egui::Key::Z => IdeAction::Undo,
        egui::Key::V => IdeAction::PasteKey,
        egui::Key::S => IdeAction::Save,
        egui::Key::F => IdeAction::Find,
        egui::Key::H => IdeAction::Replace,
        egui::Key::K => IdeAction::Hover,
        _ => return None,
    })
}

/// Who owns Ctrl-V: vim's visual block (vim on, in a vim mode) or the IDE
/// paste. Insert and Command always paste, so modeless editing — and typing
/// `:w` — never loses paste to a mode the user is not in.
#[must_use]
pub fn ctrl_v_is_block(vim: bool, mode: Mode) -> bool {
    vim && matches!(
        mode,
        Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
    )
}

/// Who owns Ctrl-H: vim's Backspace motion or the IDE replace prompt.
/// Mirrors [`ctrl_v_is_block`]: vim owns it exactly where vim keys live.
#[must_use]
pub fn ctrl_h_is_motion(vim: bool, mode: Mode) -> bool {
    vim && mode != Mode::Insert && mode != Mode::Command
}

/// Stable keyboard-focus identity for one source view. The painter response
/// id is layout-derived, so panes use this to return focus to the file after
/// header-button clicks instead of stranding keys on Save/Reload.
#[must_use]
pub fn source_focus_id(id: &str) -> egui::Id {
    egui::Id::new(("native-source-focus", id))
}

/// True when a Text event only echoes a keypress already handled as a
/// discrete vim key — e.g. the `i` that opened Insert mode must not type
/// itself. Each single-char echo consumes one recorded key, in order.
pub fn echo_of_handled_key(text: &str, handled: &mut Vec<char>) -> bool {
    if text.chars().count() != 1 {
        return false;
    }
    if let Some(pos) = handled.iter().position(|c| text.starts_with(*c)) {
        handled.remove(pos);
        return true;
    }
    false
}

/// Vim footer prompt: `:ex` commands and `/search` share the command line.
pub fn command_prompt(engine: &impl ModalEngine) -> Option<String> {
    if engine.mode() != Mode::Command {
        return None;
    }
    let line = engine.command_line();
    if line.starts_with('/') {
        Some(line.to_string())
    } else {
        Some(format!(":{line}"))
    }
}

/// Header badge text for the current vim mode.
#[must_use]
pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Visual => "VISUAL",
        Mode::VisualLine => "V-LINE",
        Mode::VisualBlock => "V-BLOCK",
        Mode::Command => "COMMAND",
    }
}

/// Header badge color for the current vim mode (readable on dark/light).
#[must_use]
pub fn mode_color(mode: Mode) -> Color32 {
    match mode {
        Mode::Normal => Color32::LIGHT_BLUE,
        Mode::Insert => Color32::LIGHT_GREEN,
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => Color32::GOLD,
        Mode::Command => Color32::ORANGE,
    }
}
