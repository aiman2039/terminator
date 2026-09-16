//! Encode keys the static binding table does not own.
//!
//! egui-winit omits `Event::Text` while Ctrl or Command is held, so unbound
//! chords such as Cmd+E would otherwise vanish. Kitty CSI u carries Super/Ctrl
//! to the PTY; Neovim maps Super to `<D-`.
use crate::backend::TerminalMode;
use crate::bindings::{BindingAction, BindingsLayout, InputKind};
use egui::{Key, Modifiers};

pub(crate) fn bytes_for_pressed_key(
    bindings: &BindingsLayout,
    key: Key,
    modifiers: Modifiers,
    terminal_mode: TerminalMode,
) -> Option<Vec<u8>> {
    match bindings.get_action(InputKind::KeyCode(key), modifiers, terminal_mode) {
        BindingAction::Char(c) => {
            let mut buf = [0; 4];
            Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        BindingAction::Esc(seq) => Some(seq.into_bytes()),
        BindingAction::Ignore => encode_unbound_modified_key(key, modifiers),
        BindingAction::Copy | BindingAction::Paste | BindingAction::LinkOpen => None,
    }
}

fn encode_unbound_modified_key(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
    if !text_omitted(modifiers) {
        return None;
    }
    let (code, suffix) = kitty_key_code(key)?;
    let mods = kitty_modifiers(modifiers);
    if mods <= 1 {
        return None;
    }
    Some(format!("\x1b[{code};{mods}{suffix}").into_bytes())
}

fn text_omitted(modifiers: Modifiers) -> bool {
    modifiers.ctrl || modifiers.command || modifiers.mac_cmd
}

fn kitty_modifiers(modifiers: Modifiers) -> u8 {
    let mut bits = 1u8;
    if modifiers.shift {
        bits += 1;
    }
    if modifiers.alt {
        bits += 2;
    }
    if modifiers.ctrl {
        bits += 4;
    }
    if modifiers.mac_cmd {
        bits += 8;
    }
    bits
}

fn kitty_key_code(key: Key) -> Option<(u32, char)> {
    functional_code(key)
        .or_else(|| fkey_code(key))
        .or_else(|| ascii_code(key).map(|code| (code, 'u')))
}

// https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions
// Navigation keys and F1–F12 retain their CSI suffix, even with Super held.
fn functional_code(key: Key) -> Option<(u32, char)> {
    Some(match key {
        Key::Escape => (27, 'u'),
        Key::Enter => (13, 'u'),
        Key::Tab => (9, 'u'),
        Key::Backspace => (127, 'u'),
        Key::Space => (32, 'u'),
        Key::Insert => (2, '~'),
        Key::Delete => (3, '~'),
        Key::ArrowLeft => (1, 'D'),
        Key::ArrowRight => (1, 'C'),
        Key::ArrowUp => (1, 'A'),
        Key::ArrowDown => (1, 'B'),
        Key::PageUp => (5, '~'),
        Key::PageDown => (6, '~'),
        Key::Home => (1, 'H'),
        Key::End => (1, 'F'),
        _ => return None,
    })
}

fn fkey_code(key: Key) -> Option<(u32, char)> {
    let n = key.name().strip_prefix('F')?.parse::<u32>().ok()?;
    Some(match n {
        1 => (1, 'P'),
        2 => (1, 'Q'),
        3 => (13, '~'),
        4 => (1, 'S'),
        5 => (15, '~'),
        6 => (17, '~'),
        7 => (18, '~'),
        8 => (19, '~'),
        9 => (20, '~'),
        10 => (21, '~'),
        11 => (23, '~'),
        12 => (24, '~'),
        13..=35 => (57363 + n, 'u'),
        _ => return None,
    })
}

fn ascii_code(key: Key) -> Option<u32> {
    if key == Key::Minus {
        return Some(u32::from(b'-'));
    }
    let mut chars = key.symbol_or_name().chars();
    let c = chars.next()?;
    if chars.next().is_some() || c.is_ascii_control() {
        return None;
    }
    Some(u32::from(c.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_letter() -> Modifiers {
        Modifiers {
            alt: false,
            ctrl: false,
            shift: false,
            mac_cmd: true,
            command: true,
        }
    }

    #[test]
    fn unbound_command_functional_keys_use_protocol_sequences() {
        let layout = BindingsLayout::default();
        // Shift keeps these chords outside the existing platform bindings.
        let modifiers = command_letter() | Modifiers::SHIFT;
        for (key, expected) in [
            (Key::Insert, "\x1b[2;10~"),
            (Key::Delete, "\x1b[3;10~"),
            (Key::ArrowLeft, "\x1b[1;10D"),
            (Key::ArrowRight, "\x1b[1;10C"),
            (Key::ArrowUp, "\x1b[1;10A"),
            (Key::ArrowDown, "\x1b[1;10B"),
            (Key::PageUp, "\x1b[5;10~"),
            (Key::PageDown, "\x1b[6;10~"),
            (Key::Home, "\x1b[1;10H"),
            (Key::End, "\x1b[1;10F"),
            (Key::F1, "\x1b[1;10P"),
            (Key::F2, "\x1b[1;10Q"),
            (Key::F3, "\x1b[13;10~"),
            (Key::F4, "\x1b[1;10S"),
            (Key::F5, "\x1b[15;10~"),
            (Key::F6, "\x1b[17;10~"),
            (Key::F7, "\x1b[18;10~"),
            (Key::F8, "\x1b[19;10~"),
            (Key::F9, "\x1b[20;10~"),
            (Key::F10, "\x1b[21;10~"),
            (Key::F11, "\x1b[23;10~"),
            (Key::F12, "\x1b[24;10~"),
            (Key::F13, "\x1b[57376;10u"),
            (Key::F35, "\x1b[57398;10u"),
        ] {
            assert_eq!(
                bytes_for_pressed_key(&layout, key, modifiers, TerminalMode::empty()),
                Some(expected.as_bytes().to_vec()),
                "{key:?}"
            );
        }
    }

    #[test]
    fn command_e_is_csi_u_when_unbound() {
        let layout = BindingsLayout::default();
        let modifiers = command_letter();
        assert_eq!(
            bytes_for_pressed_key(&layout, Key::E, modifiers, TerminalMode::empty()),
            Some(b"\x1b[101;9u".to_vec())
        );
    }

    #[test]
    fn command_shift_e_sets_the_shift_bit() {
        let mut modifiers = command_letter();
        modifiers.shift = true;
        assert_eq!(
            bytes_for_pressed_key(
                &BindingsLayout::default(),
                Key::E,
                modifiers,
                TerminalMode::empty()
            ),
            Some(b"\x1b[101;10u".to_vec())
        );
    }

    #[test]
    fn control_e_stays_enq() {
        assert_eq!(
            bytes_for_pressed_key(
                &BindingsLayout::default(),
                Key::E,
                Modifiers::CTRL,
                TerminalMode::empty()
            ),
            Some(vec![0x05])
        );
    }

    #[test]
    fn unmodified_letter_is_not_encoded() {
        assert_eq!(
            bytes_for_pressed_key(
                &BindingsLayout::default(),
                Key::E,
                Modifiers::NONE,
                TerminalMode::empty()
            ),
            None
        );
    }

    #[test]
    fn copy_chord_is_not_forwarded() {
        let modifiers = if cfg!(target_os = "macos") {
            command_letter()
        } else {
            Modifiers::SHIFT | Modifiers::COMMAND
        };
        assert_eq!(
            bytes_for_pressed_key(
                &BindingsLayout::default(),
                Key::C,
                modifiers,
                TerminalMode::empty()
            ),
            None
        );
    }

    #[test]
    fn alt_letter_is_left_to_text_events() {
        assert_eq!(encode_unbound_modified_key(Key::E, Modifiers::ALT), None);
    }
}
