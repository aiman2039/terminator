//! Row-level controls over the painted panes: the line under the pointer,
//! the pick checkboxes, and the row menu that opens the file at a line.
use super::super::super::*;
use super::{ChangedLineInput, ControlsFrame, ControlsState, PaneOutput, changed_line};

/// Width of the pick checkbox column at the left of a painted pane.
pub(super) const PICK_GUTTER: f32 = 14.0;

/// A changed line of a hunk: its row, and `(old side, line)`.
pub(super) type HunkLine = (usize, (bool, usize));

impl ControlsFrame<'_> {
    /// Document row under `pos` in `output`'s pane.
    pub(super) fn row_at(&self, output: &PaneOutput, pos: egui::Pos2) -> usize {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Row index from UI coordinates."
        )]
        let row = ((pos.y - output.inner_rect.top() + output.state.unclamped_offset().y)
            / self.row_h)
            .floor()
            .max(0.0) as usize;
        row
    }

    /// Top of `row` on screen in `output`'s pane.
    pub(super) fn row_top(&self, output: &PaneOutput, row: usize) -> f32 {
        output.inner_rect.top() + row as f32 * self.row_h - output.state.unclamped_offset().y
    }

    /// Changed lines of `hunk` on pane `side`, with their rows.
    pub(super) fn hunk_lines(&self, hunk: &diff::review::Hunk, side: usize) -> Vec<HunkLine> {
        hunk.rows(self.split)
            .filter_map(|row| Some((row, self.hunk_line_at(hunk, side, row)?)))
            .collect()
    }

    /// The changed line of `hunk` that `row` shows on pane `side`.
    pub(super) fn hunk_line_at(
        &self,
        hunk: &diff::review::Hunk,
        side: usize,
        row: usize,
    ) -> Option<(bool, usize)> {
        if !hunk.rows(self.split).contains(&row) {
            return None;
        }
        let line = changed_line(ChangedLineInput {
            doc: self.doc,
            split: self.split,
            side,
            row,
        })?;
        let (old, index) = line;
        (if old { &hunk.old } else { &hunk.new })
            .contains(&index)
            .then_some(line)
    }

    /// Empty pick boxes beside the changed lines of a hovered hunk.
    pub(super) fn paint_pick_boxes(
        &self,
        ui: &egui::Ui,
        (lines, output): (&[HunkLine], &PaneOutput),
        color: Color32,
    ) {
        let painter = ui.painter().with_clip_rect(output.inner_rect);
        for &(row, _) in lines {
            let top = self.row_top(output, row);
            let size = 9.0_f32.min(self.row_h - 2.0);
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    output.inner_rect.left() + 3.0,
                    top + (self.row_h - size) / 2.0,
                ),
                egui::vec2(size, size),
            );
            painter.rect_stroke(
                rect,
                appearance::RADIUS_XSMALL,
                egui::Stroke::new(1.0, color.gamma_multiply(0.7)),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Right-click on any painted row offers to open the file at its line
    /// (the new side's line, or the old side's for a removed line).
    pub(super) fn row_menu(&self, ui: &egui::Ui, state: &mut ControlsState<'_>) {
        let review = &mut *state.review;
        if let Some(pos) = self.pointer
            && ui.input(|input| input.pointer.secondary_clicked())
        {
            let panes = if self.editor_pane {
                self.scroll.get(..1).unwrap_or_default()
            } else {
                self.scroll
            };
            for (side, output) in panes.iter().enumerate() {
                if !output.inner_rect.contains(pos) {
                    continue;
                }
                let side = if self.split { side } else { 1 };
                if let Some(line) = self.file_line(self.row_at(output, pos), side) {
                    review.row_menu = Some(([pos.x, pos.y], line));
                }
            }
        }
        let Some(([x, y], line)) = review.row_menu else {
            return;
        };
        let mut open = true;
        let mut chosen = false;
        egui::Popup::new(
            ui.scope_id().with("diff-row-menu"),
            ui.ctx().clone(),
            egui::PopupAnchor::Position(egui::pos2(x, y)),
            ui.layer_id(),
        )
        .open_bool(&mut open)
        .style(appearance::menu_style)
        .show(|ui| {
            chosen = appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: &format!("Open in editor at line {line}"),
                icon: "FileText",
                shortcut: "",
            })
            .clicked();
        });
        if chosen {
            state.result.open_line = Some(line);
        }
        if chosen || !open {
            state.review.row_menu = None;
        }
    }

    /// 1-based file line a row shows on `side`.
    fn file_line(&self, row: usize, side: usize) -> Option<u32> {
        let line = if self.split {
            let row = self.doc.split.get(row)?;
            if side == 0 {
                row.left.as_ref()
            } else {
                row.right.as_ref()
            }
        } else {
            self.doc.unified.get(row)
        }?;
        if line.kind == diff::LineKind::Hunk {
            return None;
        }
        line.new_no.or(line.old_no)
    }
}
