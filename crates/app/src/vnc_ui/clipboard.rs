use super::*;
/// Translate local clipboard commands into the remote application's shortcuts.
/// Clipboard transfer alone does not insert text. Local modifiers must not
/// change the remote chord (for example Command must not modify Shift+Insert).
pub(super) fn remote_clipboard_events(
    event: &egui::Event,
    shortcut: VncPasteShortcut,
    held_modifiers: u8,
) -> Vec<VncInputEvent> {
    let (modifiers, key) = match (shortcut, event) {
        (VncPasteShortcut::ShiftInsert, egui::Event::Paste(_)) => (1, 0xFF63),
        (VncPasteShortcut::ShiftInsert, egui::Event::Copy) => (2, 0xFF63),
        (VncPasteShortcut::ShiftInsert, egui::Event::Cut) => (1, 0xFFFF),
        (shortcut, egui::Event::Paste(_)) => (clipboard_modifiers(shortcut), u32::from('v')),
        (shortcut, egui::Event::Copy) => (clipboard_modifiers(shortcut), u32::from('c')),
        (shortcut, egui::Event::Cut) => (clipboard_modifiers(shortcut), u32::from('x')),
        _ => return Vec::new(),
    };
    let mut events = Vec::new();
    for (keycode, down) in modifier_transitions(held_modifiers, 0) {
        events.push(VncInputEvent::Key { keycode, down });
    }
    if let egui::Event::Paste(text) = event {
        events.push(VncInputEvent::CopyText(text.to_owned()));
    }
    for (keycode, down) in modifier_transitions(0, modifiers) {
        events.push(VncInputEvent::Key { keycode, down });
    }
    events.push(VncInputEvent::Key {
        keycode: key,
        down: true,
    });
    events.push(VncInputEvent::Key {
        keycode: key,
        down: false,
    });
    for (keycode, down) in modifier_transitions(modifiers, 0)
        .into_iter()
        .chain(modifier_transitions(0, held_modifiers))
    {
        events.push(VncInputEvent::Key { keycode, down });
    }
    events
}

fn clipboard_modifiers(shortcut: VncPasteShortcut) -> u8 {
    match shortcut {
        VncPasteShortcut::ControlV => 2,
        VncPasteShortcut::ControlShiftV => 3,
        VncPasteShortcut::CommandV => 8,
        VncPasteShortcut::ShiftInsert => 1,
    }
}
