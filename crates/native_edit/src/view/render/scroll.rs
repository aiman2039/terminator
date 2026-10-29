//! After input: clipboard sync, scroll-follow, fold changes, the error
//! stripe on the scroll rail, and the status footer.
use super::SourceView;
use super::canvas::Canvas;
use super::geometry::snap;
use super::helpers::RenderHelpers;
use super::palette::EditorPalette;
use crate::doc::Buffer;
use crate::fold::Folds;
use crate::highlight::HighlightCache;
use crate::rows::{RowCells, row_hints};
use crate::view::IdeEdit;
use crate::view::command_prompt;
use crate::vim::Cursor;
use crate::vim::ModalEngine;
use egui::{Pos2, Rect, Sense, Vec2};

/// Status bar text size, horizontal padding and gap between cells.
const STATUS_FONT: f32 = 12.0;
const STATUS_PAD: f32 = 10.0;
const STATUS_GAP: f32 = 16.0;

impl<B: Buffer> SourceView<'_, B> {
    /// Yank sync (`clipboard=unnamed`): every unnamed-register write lands
    /// on the system clipboard; terminal contexts can wrap it in
    /// `clipboard::copy_sequence` instead.
    pub(super) fn sync_clipboard(&mut self, canvas: &Canvas<'_>) {
        if let Some(text) = self.engine.take_clipboard()
            && !text.is_empty()
        {
            canvas.ui.ctx().copy_text(text);
        }
    }

    /// Search and long jumps ask for the cursor row to be revealed; user
    /// scrolling never triggers this, so it cannot fight input. Other
    /// caret moves scroll just enough to keep the caret in sight.
    pub(super) fn follow_caret(&mut self, canvas: &Canvas<'_>, caret_before: Cursor) {
        let landed = self.engine.cursor();
        let caret_moved = landed != caret_before;
        if self.engine.take_scroll_request() {
            canvas.reveal_row(landed);
        } else if caret_moved {
            canvas.reveal_caret(landed);
        }
    }

    /// Fold changes, now that the row layout is free; a caret left inside
    /// a folded region opens it (`IntelliJ` unfolds where the caret goes).
    pub(super) fn apply_folds(&mut self, highlight: &mut HighlightCache) {
        let caret_line = self.engine.cursor().line;
        let fold_toggles = std::mem::take(&mut self.out.fold_toggles);
        let fold_actions = std::mem::take(&mut self.out.fold_actions);
        let folding = !fold_toggles.is_empty() || !fold_actions.is_empty();
        let folds = highlight.folds_mut();
        for line in fold_toggles {
            folds.toggle(line);
        }
        for action in fold_actions {
            match action {
                IdeEdit::Fold { all: true } => folds.fold_all(),
                IdeEdit::Fold { all: false } => folds.fold_at(caret_line),
                IdeEdit::Unfold { all: true } => folds.unfold_all(),
                _ => folds.unfold_at(caret_line),
            }
        }
        self.settle_folded_caret(folds, caret_line, folding);
    }

    fn settle_folded_caret(&mut self, folds: &mut Folds, caret_line: usize, folding: bool) {
        let Some(start) = folds.hiding(caret_line) else {
            return;
        };
        let col = self.engine.cursor().col;
        let vertical_move = self.out.vertical_move;
        if folding || vertical_move == Some(false) {
            // Folding the caret's block, or moving up into one, parks the
            // caret on its first line.
            let col = col.min(self.doc.line_text(start).chars().count());
            self.out
                .effects
                .extend(self.engine.place_cursor(self.doc, start, col));
        } else if let (Some(true), Some((_, last))) = (vertical_move, folds.region_at(start)) {
            // Moving down hops past the folded block.
            let line = last
                .saturating_add(1)
                .min(self.doc.line_count().saturating_sub(1));
            let col = col.min(self.doc.line_text(line).chars().count());
            self.out
                .effects
                .extend(self.engine.place_cursor(self.doc, line, col));
        } else {
            folds.unfold_at(caret_line);
        }
    }

    /// Error stripe: every problem as a tick on the right edge, placed by
    /// its line's share of the file (`IntelliJ`'s scrollbar marks).
    pub(super) fn paint_squiggle_rail(&self, ui: &egui::Ui, rail: Rect) {
        if self.options.squiggles.is_empty() {
            return;
        }
        let lines = self.doc.line_count().max(1);
        let painter = ui.painter().with_clip_rect(rail);
        let mut last_y = f32::NEG_INFINITY;
        for squiggle in &self.options.squiggles {
            let y = rail.top() + rail.height() * squiggle.line as f32 / lines as f32;
            if y - last_y < 2.0 {
                continue;
            }
            last_y = y;
            if !matches!(
                squiggle.severity,
                crate::lsp::Severity::Error | crate::lsp::Severity::Warning
            ) {
                continue;
            }
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rail.right() - 4.0, y), Vec2::new(4.0, 2.0)),
                0.0,
                RenderHelpers::severity_color(ui.visuals(), squiggle.severity),
            );
        }
    }

    /// Reserve the footer on every frame so entering or leaving command
    /// mode does not move the source viewport. A status band with a top
    /// hairline (`JetBrains` status bar): the command prompt owns the
    /// left, and `Ln, Col`, the host's cells and the vim mode sit
    /// right-aligned.
    pub(super) fn paint_footer(&self, ui: &mut egui::Ui) -> Option<Rect> {
        if !self.options.show_status {
            return None;
        }
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), self.geometry.footer_height),
            Sense::hover(),
        );
        let palette = self.palette(ui.visuals());
        let painter = ui.painter();
        let ppp = self.geometry.ppp;
        painter.rect_filled(rect, 0.0, palette.status_fill);
        painter.hline(
            rect.x_range(),
            snap(rect.top(), ppp) + 0.5 / ppp,
            egui::Stroke::new(1.0 / ppp, palette.status_rule),
        );
        if let Some(prompt) = command_prompt(self.engine) {
            painter.text(
                Pos2::new(rect.left() + STATUS_PAD, rect.center().y),
                egui::Align2::LEFT_CENTER,
                prompt,
                self.geometry.font.clone(),
                ui.visuals().text_color(),
            );
        }
        let font = egui::FontId::proportional(STATUS_FONT);
        let mut right = rect.right() - STATUS_PAD;
        for (text, color) in self.status_cells(&palette).into_iter().rev() {
            let galley = painter.layout_no_wrap(text, font.clone(), color);
            let size = galley.size();
            let at = Pos2::new(
                snap(right - size.x, ppp),
                snap(rect.center().y - size.y * 0.5, ppp),
            );
            painter.galley(at, galley, color);
            right = at.x - STATUS_GAP;
        }
        Some(rect)
    }

    /// Status cells, left to right: caret position, host cells, mode.
    fn status_cells(&self, palette: &EditorPalette) -> Vec<(String, egui::Color32)> {
        let cursor = self.engine.cursor();
        let mut cells = vec![(
            format!(
                "Ln {}, Col {}",
                cursor.line.saturating_add(1),
                cursor.col.saturating_add(1)
            ),
            palette.status_text,
        )];
        cells.extend(
            self.options
                .status_items
                .iter()
                .map(|item| (item.clone(), palette.status_text)),
        );
        if self.options.vim {
            let mode = self.engine.mode();
            cells.push((
                crate::view::mode_name(mode).to_owned(),
                crate::view::mode_color(mode, &palette.modes),
            ));
        }
        cells
    }
}

impl Canvas<'_> {
    /// Center the row that owns `landed`.
    fn reveal_row(&self, landed: Cursor) {
        let visual_line = self
            .rows
            .iter()
            .position(|row| row.owns_cursor(landed.line, landed.col))
            .unwrap_or(landed.line);
        let row_height = self.geometry.row_height;
        let row_y = self.origin.y + visual_line as f32 * row_height;
        self.ui.scroll_to_rect(
            Rect::from_min_size(
                Pos2::new(self.origin.x, row_y),
                Vec2::new(self.width, row_height),
            ),
            Some(egui::Align::Center),
        );
    }

    /// Keyboard and drag moves keep the caret in sight with the least
    /// scrolling (`IntelliJ`); a visible caret does not move the viewport.
    /// Rows are this frame's layout, so an edit that added lines lands on
    /// its logical line instead.
    fn reveal_caret(&self, landed: Cursor) {
        let (visual_line, cells) = self
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.owns_cursor(landed.line, landed.col))
            .map_or((landed.line, 0), |(index, row)| {
                let hints = row_hints(self.inlay_hints, row);
                let geometry = RowCells {
                    text: &row.text,
                    hints: &hints,
                };
                (
                    index,
                    geometry.caret_x(landed.col.saturating_sub(row.start)),
                )
            });
        let advance = self.geometry.advance;
        let caret = Rect::from_min_size(
            Pos2::new(
                self.text_x(cells),
                self.origin.y + visual_line as f32 * self.geometry.row_height,
            ),
            Vec2::new(advance, self.geometry.row_height),
        );
        self.ui
            .scroll_to_rect(caret.expand2(Vec2::new(advance * 2.0, 0.0)), None);
    }
}
