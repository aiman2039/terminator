//! Diff-local keys while the diff is the active pane and no text widget
//! has focus: `]c` / `[c` walk changes, `s` / `u` / `x` stage, unstage, or
//! discard the selected change (discard still asks), Space and
//! Shift+Up/Down pick lines, Esc clears picks, Alt+Up/Down walk files,
//! and Enter confirms a pending discard.
use super::super::super::*;
use super::{DiffFrame, DiffPass};
use diff::actions::Kind;

impl App {
    pub(super) fn diff_local_keys(ui: &egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        // Any focused widget (the split editor too) owns its keys.
        if !frame.active || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let keys: Vec<DiffKey> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Text(text) => Some(DiffKey::Text(text.clone())),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(DiffKey::Press(*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for key in keys {
            match key {
                DiffKey::Text(text) => diff_text_key(frame, pass, &text),
                DiffKey::Press(key, modifiers) => diff_press(frame, pass, key, modifiers),
            }
        }
    }
}

/// The parts of an input event the diff keys read.
enum DiffKey {
    Text(String),
    Press(egui::Key, egui::Modifiers),
}

/// Typed characters: `]` / `[` then `c`, and the action letters.
fn diff_text_key(frame: &DiffFrame<'_>, pass: &mut DiffPass, text: &str) {
    let bracket = pass.review.bracket.take();
    match (text, bracket) {
        ("]", _) => pass.review.bracket = Some(true),
        ("[", _) => pass.review.bracket = Some(false),
        ("c", Some(forward)) => pass.step_change(frame, forward),
        ("s", None) if !frame.staged => request(frame, pass, Kind::Stage),
        ("u", None) if frame.staged => request(frame, pass, Kind::Unstage),
        ("x", None) if !frame.staged => request(frame, pass, Kind::Discard),
        _ => {}
    }
}

/// Ask for `kind` on the selected change; picked lines in it narrow the
/// action. Discard only opens the confirmation.
fn request(frame: &DiffFrame<'_>, pass: &mut DiffPass, kind: Kind) {
    if frame.enabled
        && !pass.review.unsaved_edits()
        && let Some(index) = pass.review.selected.filter(|index| *index < frame.count)
    {
        pass.action = Some((index, kind));
    }
}

fn diff_press(frame: &DiffFrame<'_>, pass: &mut DiffPass, key: egui::Key, mods: egui::Modifiers) {
    let plain = mods.is_none();
    let review = &mut pass.review;
    match key {
        egui::Key::Escape if plain => {
            review.clear_picks();
            review.confirm_discard = None;
        }
        egui::Key::Enter if plain => {
            if let Some(index) = review.confirm_discard {
                pass.confirmed = Some(index);
            }
        }
        egui::Key::Space if plain => toggle_line(frame, pass),
        egui::Key::ArrowDown | egui::Key::ArrowUp => {
            let forward = key == egui::Key::ArrowDown;
            if mods == egui::Modifiers::ALT {
                pass.navigate.clone_from(if forward {
                    &frame.next_file
                } else {
                    &frame.prev_file
                });
            } else if mods == egui::Modifiers::SHIFT
                && let Some(doc) = &frame.doc
                && let Some(index) = review.selected
                && let Some(hunk) = doc.hunks.get(index)
            {
                review.extend_pick(index, hunk, forward);
            }
        }
        _ => {}
    }
}

/// Space: toggle the changed line under the pointer, or the next line of
/// the selected change when the pointer is elsewhere.
fn toggle_line(frame: &DiffFrame<'_>, pass: &mut DiffPass) {
    let review = &mut pass.review;
    if let Some((hunk, old, line)) = review.hover_line {
        review.toggle_line(hunk, old, line);
    } else if let Some(doc) = &frame.doc
        && let Some(index) = review.selected
        && let Some(hunk) = doc.hunks.get(index)
    {
        review.extend_pick(index, hunk, true);
    }
}
