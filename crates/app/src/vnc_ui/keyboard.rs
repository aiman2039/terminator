use super::*;
/// Remote modifier keysyms: one X11 keysym per bit of
/// [`VncViewState::modifier_bits`].
pub(super) const MODIFIER_KEYSYMS: [(u8, u32); 4] = [
    (1, 0xFFE1), // Shift_L
    (2, 0xFFE3), // Control_L
    (4, 0xFFE9), // Alt_L
    (8, 0xFFEB), // Meta_L (Command on macOS)
];

/// Local modifier state reduced to the remote's four keys.
pub(super) fn modifier_bits(modifiers: egui::Modifiers) -> u8 {
    let mut bits = 0u8;
    if modifiers.shift {
        bits |= 1;
    }
    if modifiers.ctrl {
        bits |= 2;
    }
    if modifiers.alt {
        bits |= 4;
    }
    // `command` mirrors Ctrl away from macOS; on macOS it is Command.
    if modifiers.command && !modifiers.ctrl {
        bits |= 8;
    }
    bits
}

/// Remote key transitions between two modifier snapshots: the remote sees
/// exactly the modifiers that changed, never a stale held key.
pub(super) fn modifier_transitions(before: u8, after: u8) -> Vec<(u32, bool)> {
    let mut transitions = Vec::new();
    for (bit, keysym) in MODIFIER_KEYSYMS {
        let was_down = before & bit != 0;
        let is_down = after & bit != 0;
        if was_down != is_down {
            transitions.push((keysym, is_down));
        }
    }
    transitions
}

/// Letter and digit keysyms for accelerator chords: egui suppresses the
/// text event while Ctrl/Command is held, so the letter has to travel as a
/// key event or the remote shortcut never fires.
pub(super) fn accelerator_keysym(key: egui::Key) -> Option<u32> {
    use egui::Key;
    let letter = match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        _ => return None,
    };
    Some(u32::from(letter))
}

pub(super) fn egui_key_to_x11(key: egui::Key) -> Option<u32> {
    use egui::Key;
    match key {
        Key::Backspace => Some(0xFF08),
        Key::Tab => Some(0xFF09),
        Key::Enter => Some(0xFF0D),
        Key::Escape => Some(0xFF1B),
        Key::Insert => Some(0xFF63),
        Key::Delete => Some(0xFFFF),
        Key::Home => Some(0xFF50),
        Key::ArrowLeft => Some(0xFF51),
        Key::ArrowUp => Some(0xFF52),
        Key::ArrowRight => Some(0xFF53),
        Key::ArrowDown => Some(0xFF54),
        Key::PageUp => Some(0xFF55),
        Key::PageDown => Some(0xFF56),
        Key::End => Some(0xFF57),
        Key::F1 => Some(0xFFBE),
        Key::F2 => Some(0xFFBF),
        Key::F3 => Some(0xFFC0),
        Key::F4 => Some(0xFFC1),
        Key::F5 => Some(0xFFC2),
        Key::F6 => Some(0xFFC3),
        Key::F7 => Some(0xFFC4),
        Key::F8 => Some(0xFFC5),
        Key::F9 => Some(0xFFC6),
        Key::F10 => Some(0xFFC7),
        Key::F11 => Some(0xFFC8),
        Key::F12 => Some(0xFFC9),
        _ => None,
    }
}

pub(super) fn text_keysym(ch: char) -> Option<u32> {
    match ch {
        '\n' | '\r' => Some(0xFF0D),
        '\t' => Some(0xFF09),
        c if c.is_control() => None,
        c => {
            let code = u32::from(c);
            Some(if code <= 255 {
                code
            } else {
                0x0100_0000 | code
            })
        }
    }
}
