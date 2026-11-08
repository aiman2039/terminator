//! egui keys to Linux evdev key codes (linux/input-event-codes.h). The host
//! maps codes with its own keymap (US by default), so the physical key
//! position is sent, not the character.

use eframe::egui::{Key, Modifiers};

pub(super) const LEFT_SHIFT: u32 = 42;
pub(super) const LEFT_CTRL: u32 = 29;
pub(super) const LEFT_ALT: u32 = 56;

/// The evdev code of a physical key, or `None` for keys the host cannot
/// take (media keys, Fn) and modifiers (sent from [`modifier_changes`]).
pub(super) fn evdev(key: Key) -> Option<u32> {
    Some(match key {
        Key::Escape => 1,
        Key::Num1 => 2,
        Key::Num2 => 3,
        Key::Num3 => 4,
        Key::Num4 => 5,
        Key::Num5 => 6,
        Key::Num6 => 7,
        Key::Num7 => 8,
        Key::Num8 => 9,
        Key::Num9 => 10,
        Key::Num0 => 11,
        Key::Minus => 12,
        Key::Equals | Key::Plus => 13,
        Key::Backspace => 14,
        Key::Tab => 15,
        Key::Q => 16,
        Key::W => 17,
        Key::E => 18,
        Key::R => 19,
        Key::T => 20,
        Key::Y => 21,
        Key::U => 22,
        Key::I => 23,
        Key::O => 24,
        Key::P => 25,
        Key::OpenBracket | Key::OpenCurlyBracket => 26,
        Key::CloseBracket | Key::CloseCurlyBracket => 27,
        Key::Enter => 28,
        Key::A => 30,
        Key::S => 31,
        Key::D => 32,
        Key::F => 33,
        Key::G => 34,
        Key::H => 35,
        Key::J => 36,
        Key::K => 37,
        Key::L => 38,
        Key::Semicolon | Key::Colon => 39,
        Key::Quote => 40,
        Key::Backtick => 41,
        Key::Backslash | Key::Pipe => 43,
        Key::Z => 44,
        Key::X => 45,
        Key::C => 46,
        Key::V => 47,
        Key::B => 48,
        Key::N => 49,
        Key::M => 50,
        Key::Comma => 51,
        Key::Period => 52,
        Key::Slash | Key::Questionmark => 53,
        Key::Space => 57,
        Key::CapsLock => 58,
        Key::F1 => 59,
        Key::F2 => 60,
        Key::F3 => 61,
        Key::F4 => 62,
        Key::F5 => 63,
        Key::F6 => 64,
        Key::F7 => 65,
        Key::F8 => 66,
        Key::F9 => 67,
        Key::F10 => 68,
        Key::F11 => 87,
        Key::F12 => 88,
        Key::IntlBackslash => 86,
        Key::Home => 102,
        Key::ArrowUp => 103,
        Key::PageUp => 104,
        Key::ArrowLeft => 105,
        Key::ArrowRight => 106,
        Key::End => 107,
        Key::ArrowDown => 108,
        Key::PageDown => 109,
        Key::Insert => 110,
        Key::Delete => 111,
        Key::Pause => 119,
        Key::Menu => 127,
        Key::PrintScreen => 99,
        _ => return None,
    })
}

/// Modifier keys, pressed or released, that turn `held` into `now`. macOS
/// Command acts as Control, so Cmd+C copies in the remote app too.
pub(super) fn modifier_changes(held: Modifiers, now: Modifiers) -> Vec<(u32, bool)> {
    let ctrl = |m: Modifiers| m.ctrl || m.mac_cmd;
    [
        (LEFT_SHIFT, held.shift, now.shift),
        (LEFT_CTRL, ctrl(held), ctrl(now)),
        (LEFT_ALT, held.alt, now.alt),
    ]
    .into_iter()
    .filter(|(_, before, after)| before != after)
    .map(|(code, _, after)| (code, after))
    .collect()
}
