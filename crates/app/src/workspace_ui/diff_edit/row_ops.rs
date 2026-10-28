use super::super::super::*;

/// Structural edit requested from a unified row editor.
pub(in super::super) enum RowOp {
    /// Enter: split the line at the cursor into head/tail.
    Split {
        line: usize,
        head: String,
        tail: String,
    },
    /// Backspace at column 0: join this line onto the previous one.
    JoinPrev { line: usize },
    /// Delete at end of line: pull the next line up.
    JoinNext { line: usize },
}

/// Apply a structural edit to whole-buffer text, preserving its line ending
/// style. Out-of-range lines leave the text untouched.
pub(in super::super) fn apply_row_op(text: &str, op: &RowOp) -> String {
    let sep = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split(sep).map(str::to_owned).collect();
    match op {
        RowOp::Split { line, head, tail } => {
            if let Some(slot) = lines.get_mut(*line) {
                slot.clone_from(head);
                lines.insert(line.saturating_add(1), tail.clone());
            }
        }
        RowOp::JoinPrev { line } => {
            if *line > 0 && *line < lines.len() {
                let current = lines.remove(*line);
                if let Some(prev) = line.checked_sub(1).and_then(|prev| lines.get_mut(prev)) {
                    prev.push_str(&current);
                }
            }
        }
        RowOp::JoinNext { line } => {
            if line.saturating_add(1) < lines.len() {
                let next = lines.remove(line.saturating_add(1));
                if let Some(current) = lines.get_mut(*line) {
                    current.push_str(&next);
                }
            }
        }
    }
    lines.join(sep)
}

/// Record the pre-edit text as one undo unit. Edits within 0.8s of the last
/// snapshot belong to the same burst and don't add another unit.
pub(super) fn record_undo(buffer: &mut diff::review::Edit, old_text: String, now: f64) {
    if now - buffer.burst > 0.8 || buffer.undo.is_empty() {
        buffer.undo.push(old_text);
        if buffer.undo.len() > 100 {
            buffer.undo.remove(0);
        }
    }
    buffer.burst = now;
    buffer.redo.clear();
}

/// Undo one unit. Returns true when the text changed.
pub(super) fn apply_undo(buffer: &mut diff::review::Edit) -> bool {
    let Some(previous) = buffer.undo.pop() else {
        return false;
    };
    let current = std::mem::replace(&mut buffer.text, previous);
    buffer.stale = true;
    buffer.redo.push(current);
    buffer.burst = 0.0;
    true
}

/// Redo one unit. Returns true when the text changed.
pub(super) fn apply_redo(buffer: &mut diff::review::Edit) -> bool {
    let Some(next) = buffer.redo.pop() else {
        return false;
    };
    let current = std::mem::replace(&mut buffer.text, next);
    buffer.stale = true;
    buffer.undo.push(current);
    buffer.burst = 0.0;
    true
}

/// Buffer-level undo/redo, but only when a unified row editor holds
/// focus (never steal Cmd+Z from e.g. the commit message), and
/// consumed here so the row editor does not also apply its own
/// per-line undo.
pub(super) fn buffer_undo_keys(ui: &mut egui::Ui, buffer: &mut diff::review::Edit, key: &str) {
    let command_z = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Z,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command || modifiers.ctrl
            )
        })
    });
    if !command_z {
        return;
    }
    let focused = ui.memory(|memory| memory.focused());
    let line_count = buffer.text.split_inclusive('\n').count();
    let ours = focused.is_some_and(|id| {
        (1..=line_count.saturating_add(2))
            .any(|n| id == egui::Id::unique((key, "unified-line", u32::try_from(n).unwrap_or(1))))
    });
    if !ours {
        return;
    }
    let mut shift_command = egui::Modifiers::COMMAND;
    shift_command.shift = true;
    let redo_pressed = ui.input_mut(|input| input.consume_key(shift_command, egui::Key::Z));
    let undo_pressed = !redo_pressed
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
    if redo_pressed {
        apply_redo(buffer);
    } else if undo_pressed {
        apply_undo(buffer);
    }
}
