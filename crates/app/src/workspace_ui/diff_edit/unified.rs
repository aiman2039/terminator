use super::super::super::*;
use super::super::diff_paint::{
    DiffColors, DiffContentWidthInput, DiffGutter, DiffGutterPaint, DiffMetrics, DiffPaint,
    FillDiffRowInput, diff_content_width, diff_row_style, fill_diff_row, gutter_cols,
    paint_diff_gutter, paint_diff_line,
};
use super::row_ops::{RowOp, apply_redo, apply_row_op, apply_undo, record_undo};
use super::{
    HighlightedJobInput, MAX_HIGHLIGHT_BYTES, bg_to_layout, ensure_cache, highlighted_job,
    replace_line,
};
use diff::review::LiveRow;
use std::sync::Arc;

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// The unified diff is editable everywhere: rows are the live diff of the
/// whole file (see `live_rows`), every buffer row hosts its editor at its
/// own row, deleted rows stay painted in place, and changed blocks carry a
/// full-width background. No hunk windows, no double-click, no mode.
pub(crate) struct PaintUnifiedEditorInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) buffer: &'a mut diff::review::Edit,
    pub(crate) path: &'a std::path::Path,
    pub(crate) colors: DiffColors,
    pub(crate) key: &'a str,
}

/// Per-frame values every unified row reads. Borrows the buffer text
/// read-only; row edits are collected and applied after the rows paint.
struct UnifiedFrame<'a> {
    text: &'a str,
    key: &'a str,
    colors: DiffColors,
    metrics: DiffMetrics,
    width: f32,
    font: egui::FontId,
    fallback: Color32,
    left_spans: &'a [Vec<diff::DiffSpan>],
    spans: &'a [Vec<diff::DiffSpan>],
    valid: &'a [bool],
    buffer_lines: usize,
}

/// Edits the rows asked for this frame.
#[derive(Default)]
struct UnifiedEdits {
    pending: Vec<(usize, String)>,
    row_op: Option<RowOp>,
    focus_after: Option<(u32, usize)>,
}

/// One buffer line shown as an editable unified row.
struct EditCell {
    line_index: usize,
    new_no: u32,
    editor_id: egui::Id,
    buffer_line: String,
    line_spans: Vec<diff::DiffSpan>,
    line_valid: bool,
    bg: Color32,
}

pub(in super::super) fn paint_unified_editor(input: PaintUnifiedEditorInput<'_>) -> PaneOutput {
    let PaintUnifiedEditorInput {
        ui,
        doc,
        buffer,
        path,
        colors,
        key,
    } = input;

    UnifiedFrame::undo_keys(ui, buffer, key);
    if buffer.left_spans.is_none() {
        buffer.left_spans = Some(Arc::new(
            if buffer.document.left_text.len() > MAX_HIGHLIGHT_BYTES {
                Vec::new()
            } else {
                diff::highlight_spans(path, &buffer.document.left_text)
            },
        ));
    }
    let left_spans = buffer.left_spans.clone().unwrap_or_default();
    let (spans, valid, _tints, rows) = ensure_cache(buffer, path, colors);
    let buffer_lines = buffer.text.split_inclusive('\n').count();
    let left_lines = buffer.document.left_text.split_inclusive('\n').count();
    let digits = u32::try_from(left_lines.max(buffer_lines).to_string().len()).unwrap_or(4);
    let metrics = DiffMetrics::measure(ui, digits.max(1));
    let content = diff_content_width(DiffContentWidthInput {
        ui,
        doc,
        split: false,
        metrics: &metrics,
        scroll_key: key,
    });
    let frame = UnifiedFrame {
        text: &buffer.text,
        key,
        colors,
        width: content.max(ui.available_width()),
        font: egui::TextStyle::Monospace.resolve(ui.style()),
        fallback: colors.text,
        metrics,
        left_spans: &left_spans,
        spans: &spans,
        valid: &valid,
        buffer_lines,
    };
    let mut edits = UnifiedEdits::default();
    let output = frame.show_rows(ui, &rows, &mut edits);
    edits.apply(ui, buffer);
    edits.restore_focus(ui, key);
    output
}

impl UnifiedFrame<'_> {
    /// Buffer-level undo/redo, but only when a unified row editor holds
    /// focus (never steal Cmd+Z from e.g. the commit message), and
    /// consumed here so the row editor does not also apply its own
    /// per-line undo.
    fn undo_keys(ui: &mut egui::Ui, buffer: &mut diff::review::Edit, key: &str) {
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
            (1..=line_count.saturating_add(2)).any(|n| {
                id == egui::Id::unique((key, "unified-line", u32::try_from(n).unwrap_or(1)))
            })
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

    fn show_rows(
        &self,
        ui: &mut egui::Ui,
        rows: &[LiveRow],
        edits: &mut UnifiedEdits,
    ) -> PaneOutput {
        egui::ScrollArea::both()
            .id_salt((self.key, "unified-editor"))
            .auto_shrink([false, false])
            .show_rows(ui, self.metrics.row_h, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_min_width(self.width);
                for index in range {
                    let Some(row) = rows.get(index) else {
                        continue;
                    };
                    match row {
                        LiveRow::Gone { old, text } => self.paint_gone_row(ui, *old, text),
                        LiveRow::Edit { old, new, changed } => {
                            self.edit_row(ui, (*old, *new, changed.is_some()), edits);
                        }
                    }
                }
            })
    }

    /// A line the buffer dropped, painted in place as a deletion.
    fn paint_gone_row(&self, ui: &mut egui::Ui, old: usize, text: &str) {
        let fallback = self.fallback;
        let line = diff::DiffLine {
            kind: diff::LineKind::Delete,
            old_no: u32::try_from(old).ok().map(|n| n.saturating_add(1)),
            new_no: None,
            spans: self
                .left_spans
                .get(old)
                .cloned()
                .filter(|spans| spans.iter().map(|s| s.text.as_str()).collect::<String>() == text)
                .unwrap_or_else(|| {
                    vec![diff::DiffSpan {
                        text: text.to_owned(),
                        rgb: [fallback.r(), fallback.g(), fallback.b()],
                        intra: diff::Intra::None,
                    }]
                }),
        };
        paint_diff_line(
            ui,
            DiffPaint {
                width: self.width,
                line: &line,
                gutter: DiffGutter::Unified,
                colors: self.colors,
                metrics: &self.metrics,
            },
        );
    }

    /// A buffer line: painted gutter and background, with a single-line
    /// editor over its code column.
    fn edit_row(
        &self,
        ui: &mut egui::Ui,
        (old, line_index, changed): (Option<usize>, usize, bool),
        edits: &mut UnifiedEdits,
    ) {
        let kind = if changed {
            diff::LineKind::Insert
        } else {
            diff::LineKind::Equal
        };
        let line = diff::DiffLine {
            kind,
            old_no: old
                .and_then(|n| u32::try_from(n).ok())
                .map(|n| n.saturating_add(1)),
            new_no: u32::try_from(line_index).ok().map(|n| n.saturating_add(1)),
            spans: Vec::new(),
        };
        let (bg, sign) = diff_row_style(kind, self.colors);
        let bg = if changed {
            super::super::diff_paint::tint(self.colors.added, 95)
        } else {
            bg
        };
        let code_rect = self.paint_row_chrome(ui, &line, (bg, sign));
        let new_no = line.new_no.unwrap_or(1);
        let cell = EditCell {
            line_index,
            new_no,
            editor_id: egui::Id::unique((self.key, "unified-line", new_no)),
            buffer_line: self
                .text
                .split_inclusive('\n')
                .nth(line_index)
                .map(|part| part.trim_end_matches(['\n', '\r']).to_owned())
                .unwrap_or_default(),
            line_spans: self.spans.get(line_index).cloned().unwrap_or_default(),
            line_valid: self.valid.get(line_index).copied().unwrap_or(false),
            bg,
        };
        let mut local = cell.buffer_line.clone();
        if let Some((op, focus)) = self.edit_cell(ui, (&cell, code_rect), &mut local) {
            edits.row_op = Some(op);
            edits.focus_after = Some(focus);
        }
        if local != cell.buffer_line {
            edits.pending.push((line_index, local));
        }
    }

    /// Row background and gutter. Returns the code column's rect.
    fn paint_row_chrome(
        &self,
        ui: &mut egui::Ui,
        line: &diff::DiffLine,
        (bg, sign): (Color32, &'static str),
    ) -> egui::Rect {
        let metrics = &self.metrics;
        let cols = gutter_cols(metrics, DiffGutter::Unified);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(self.width, metrics.row_h), egui::Sense::hover());
        fill_diff_row(FillDiffRowInput {
            ui,
            rect,
            gutter_w: cols.code,
            bg,
        });
        paint_diff_gutter(
            ui,
            DiffGutterPaint {
                rect,
                cols,
                line,
                sign,
                colors: self.colors,
                metrics,
            },
        );
        egui::Rect::from_min_max(egui::pos2(rect.left() + cols.code, rect.top()), rect.max)
    }

    fn edit_cell(
        &self,
        ui: &mut egui::Ui,
        (cell, code_rect): (&EditCell, egui::Rect),
        local: &mut String,
    ) -> Option<(RowOp, (u32, usize))> {
        let editor_id = cell.editor_id;
        // Virtualized rows can lose the text widget's click to the dock.
        // Hold focus across press and release so the dock cannot clear it
        // before the next keyboard event.
        let row_pressed = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        ..
                    } if code_rect.contains(*pos)
                )
            })
        });
        if row_pressed {
            ui.memory_mut(|memory| memory.request_focus(editor_id));
        }
        // Enter surrenders a single-line editor's focus during `show`, so
        // capture focus before it runs.
        let focused_before = ui.memory(|memory| memory.focused() == Some(editor_id));
        let mut structural = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(code_rect), |ui| {
            // egui 0.37 floors a text edit at `interact_size.y`. Pin it to
            // the row so the row pitch stays the font line height instead of
            // the toolbar height.
            ui.spacing_mut().interact_size.y = self.metrics.row_h;
            let editor = self.row_editor(ui, cell, local);
            if row_pressed {
                editor.response.request_focus();
            }
            // Structural editing, like a real text editor: Enter splits,
            // Backspace/Delete join lines. TextEdit already applied any
            // in-line change, so an unchanged line with the cursor at an
            // edge means the key was structural, not textual.
            if focused_before
                && *local == cell.buffer_line
                && let Some((op, focus)) = self.structural_key(ui, cell, &editor)
            {
                structural = Some((op, focus));
            }
            if editor.response.has_focus() {
                self.arrow_focus(ui, cell);
            }
        });
        structural
    }

    fn row_editor(
        &self,
        ui: &mut egui::Ui,
        cell: &EditCell,
        local: &mut String,
    ) -> egui::text_edit::TextEditOutput {
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            ui.ctx().fonts_mut(|fonts| {
                fonts.layout_job(highlighted_job(HighlightedJobInput {
                    text: text.as_str(),
                    spans: std::slice::from_ref(&cell.line_spans),
                    valid: &[cell.line_valid],
                    tints: &[bg_to_layout(cell.bg)],
                    font: self.font.clone(),
                    fallback: self.fallback,
                    wrap_width,
                }))
            })
        };
        let editor = egui::TextEdit::singleline(local)
            .id(cell.editor_id)
            .font(egui::TextStyle::Monospace)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .layouter(&mut layouter)
            .desired_width(f32::INFINITY)
            .show(ui);
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("diff-unified-edit-{}", cell.new_no),
            editor.response.rect,
        );
        editor
    }

    /// Enter, Backspace at column 0, or Delete at the end of the line:
    /// the row edit and where focus lands after it.
    fn structural_key(
        &self,
        ui: &egui::Ui,
        cell: &EditCell,
        editor: &egui::text_edit::TextEditOutput,
    ) -> Option<(RowOp, (u32, usize))> {
        let (line_index, new_no) = (cell.line_index, cell.new_no);
        let local = &cell.buffer_line;
        let range = editor.state.cursor.char_range();
        let (min, max) = range.map_or((0, 0), |r| {
            (
                r.primary.index.0.min(r.secondary.index.0),
                r.primary.index.0.max(r.secondary.index.0),
            )
        });
        let char_len = local.chars().count();
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let head: String = local.chars().take(min).collect();
            let tail: String = local.chars().skip(max).collect();
            return Some((
                RowOp::Split {
                    line: line_index,
                    head,
                    tail,
                },
                (new_no.saturating_add(1), 0),
            ));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Backspace))
            && min == 0
            && max == 0
            && line_index > 0
        {
            let previous_len = self
                .text
                .split_inclusive('\n')
                .nth(line_index.saturating_sub(1))
                .map_or(0, |part| {
                    part.trim_end_matches(['\n', '\r']).chars().count()
                });
            return Some((
                RowOp::JoinPrev { line: line_index },
                (new_no.saturating_sub(1), previous_len),
            ));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) && min == char_len && max == char_len {
            return Some((RowOp::JoinNext { line: line_index }, (new_no, char_len)));
        }
        None
    }

    /// Arrow keys move focus to the neighboring row's editor.
    fn arrow_focus(&self, ui: &egui::Ui, cell: &EditCell) {
        let new_no = cell.new_no;
        let target = if ui.input(|i| i.key_pressed(egui::Key::ArrowDown))
            && cell.line_index.saturating_add(1) < self.buffer_lines
        {
            Some(new_no.saturating_add(1))
        } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) && new_no > 1 {
            Some(new_no.saturating_sub(1))
        } else {
            None
        };
        if let Some(next) = target {
            ui.memory_mut(|memory| {
                memory.request_focus(egui::Id::unique((self.key, "unified-line", next)));
            });
        }
    }
}

impl UnifiedEdits {
    /// Apply line edits and the structural edit as one undo unit.
    fn apply(&mut self, ui: &egui::Ui, buffer: &mut diff::review::Edit) {
        if self.pending.is_empty() && self.row_op.is_none() {
            return;
        }
        let old_text = buffer.text.clone();
        for (line_index, text) in self.pending.drain(..) {
            buffer.text = replace_line(&buffer.text, line_index, &text);
        }
        if let Some(op) = &self.row_op {
            buffer.text = apply_row_op(&buffer.text, op);
        }
        buffer.stale = true;
        if buffer.text != old_text {
            let now = ui.input(|input| input.time);
            record_undo(buffer, old_text, now);
        }
    }

    /// Put the cursor where a structural edit left it.
    fn restore_focus(&self, ui: &egui::Ui, key: &str) {
        let Some((line_no, char_pos)) = self.focus_after else {
            return;
        };
        let id = egui::Id::unique((key, "unified-line", line_no));
        ui.memory_mut(|memory| memory.request_focus(id));
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state
                .cursor
                .set_char_range(Some(egui::text_selection::CCursorRange::one(
                    egui::text::CCursor::new(char_pos),
                )));
            state.store(ui.ctx(), id);
        }
    }
}
