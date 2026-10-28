//! Row painting: indent guides, selections, match highlights, text, fold
//! placeholders, diagnostic underlines and carets.
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use super::helpers::{RenderHelpers, SelectionSpanForLineInput};
use super::row_paint::RowPaint;
use crate::doc::Buffer;
use crate::highlight::Span;
use crate::rows::{display_x, row_hints};
use crate::vim::ModalEngine;
use crate::vim::{Caret, Cursor, Mode};
use egui::{Color32, Pos2, Rect, Sense, Vec2};

/// Caret and selection state the rows paint, read once before painting.
pub(super) struct RowMarks {
    /// Selection in line/col space, ordered.
    selection: Option<(Cursor, Cursor)>,
    /// A visual block paints its rectangle instead: (top, bottom, left,
    /// right+1).
    block: Option<(usize, usize, usize, usize)>,
    /// Extra carets (multi-caret editing) paint like the primary one.
    extra_carets: Vec<Caret>,
    cursor: Cursor,
    cursor_visible: bool,
    /// Occurrences of the identifier at the caret, and the bracket pair
    /// around it: both paint as quiet backgrounds/outlines.
    caret_word: Option<String>,
    brackets: Option<(Cursor, Cursor)>,
    occurrence_fill: Color32,
    bracket_stroke: egui::Stroke,
    guide_color: Color32,
    /// A bar caret sits before a hint; a block covers the char after it.
    bar: bool,
}

impl<B: Buffer> SourceView<'_, B> {
    pub(super) fn row_marks(&self, canvas: &Canvas<'_>) -> RowMarks {
        let engine = &*self.engine;
        let visuals = canvas.ui.visuals();
        let selection = engine.selection().map(|(a, b)| {
            let (a_idx, b_idx) = (
                RenderHelpers::line_col_ord(a),
                RenderHelpers::line_col_ord(b),
            );
            if a_idx <= b_idx { (a, b) } else { (b, a) }
        });
        let cursor_visible = !matches!(engine.mode(), Mode::Command);
        let caret_word = (cursor_visible && selection.is_none())
            .then(|| engine.word_at_caret(&*self.doc))
            .flatten();
        RowMarks {
            selection,
            block: engine.block_span(),
            extra_carets: engine.extra_carets().to_vec(),
            cursor: engine.cursor(),
            cursor_visible,
            caret_word,
            brackets: cursor_visible
                .then(|| engine.matching_bracket(&*self.doc))
                .flatten(),
            occurrence_fill: visuals.selection.bg_fill.gamma_multiply(0.45),
            bracket_stroke: egui::Stroke::new(1.0, visuals.weak_text_color()),
            // Indent guides: blank lines keep the guides of the line above.
            guide_color: visuals
                .widgets
                .noninteractive
                .bg_stroke
                .color
                .gamma_multiply(0.6),
            bar: engine.mode() == Mode::Insert || !self.options.vim,
        }
    }

    pub(super) fn paint_rows(&mut self, canvas: &Canvas<'_>, marks: &RowMarks) {
        let row_height = self.geometry.row_height;
        let mut guide_cells = 0usize;
        for (visual_line, visual) in canvas.visible_rows() {
            let hints = row_hints(canvas.inlay_hints, visual);
            let y = canvas.origin.y + visual_line as f32 * row_height;
            let row = PaintRow::new(visual, &hints, y);
            if y + row_height < canvas.clip.top() || y > canvas.clip.bottom() {
                continue;
            }
            self.paint_gutter(canvas, &row);
            guide_cells = canvas.paint_indent_guides(&row, guide_cells, marks.guide_color);
            self.paint_selections(canvas, &row, marks);
            self.paint_match_marks(canvas, &row, marks);
            self.paint_row_text(canvas, &row);
            self.paint_fold_placeholder(canvas, &row);
            self.paint_row_squiggles(canvas, &row);
            self.paint_extra_carets(canvas, &row, marks);
            self.paint_caret(canvas, &row, marks);
        }
    }

    /// Selection background for this row: the primary selection (or
    /// visual block), then every extra caret's selection.
    fn paint_selections(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>, marks: &RowMarks) {
        let (line, line_len) = (row.line(), row.visual.line_len);
        let span = match marks.block {
            Some((top, bottom, left, right)) if line >= top && line <= bottom => {
                (left < right.min(line_len)).then(|| (left, right.min(line_len)))
            }
            _ => marks.selection.and_then(|(a, b)| {
                RenderHelpers::selection_span_for_line(SelectionSpanForLineInput {
                    a,
                    b,
                    line,
                    line_len,
                    inclusive: self.options.vim,
                })
            }),
        };
        let fill = canvas.ui.visuals().selection.bg_fill;
        if let Some(rect) = span.and_then(|(start, end)| row.span_rect(canvas, start, end)) {
            canvas.painter.rect_filled(rect, 0.0, fill);
        }
        for caret in &marks.extra_carets {
            let span = caret.selection().and_then(|(a, b)| {
                RenderHelpers::selection_span_for_line(SelectionSpanForLineInput {
                    a,
                    b,
                    line,
                    line_len,
                    inclusive: false,
                })
            });
            if let Some(rect) = span.and_then(|(start, end)| row.span_rect(canvas, start, end)) {
                canvas.painter.rect_filled(rect, 0.0, fill);
            }
        }
    }

    /// Caret-word occurrences, find-bar matches (all softly, the selected
    /// one strongly) and the bracket pair around the caret.
    fn paint_match_marks(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>, marks: &RowMarks) {
        let (visual, line) = (row.visual, row.line());
        if let Some(word) = marks.caret_word.as_deref() {
            for (start, len) in RenderHelpers::word_occurrences(&visual.text, word) {
                let rect = row.cell_rect(canvas, visual.start.saturating_add(start), len);
                canvas.painter.rect_filled(rect, 2.0, marks.occurrence_fill);
            }
        }
        let find_matches = &self.options.find_matches;
        let from = find_matches.partition_point(|(match_line, _, _)| *match_line < line);
        for (index, (_, start, end)) in find_matches
            .iter()
            .enumerate()
            .skip(from)
            .take_while(|(_, (match_line, _, _))| *match_line == line)
        {
            let (start, end) = ((*start).max(visual.start), (*end).min(visual.end));
            if start >= end {
                continue;
            }
            let alpha = if self.options.find_current == Some(index) {
                0.9
            } else {
                0.4
            };
            canvas.painter.rect_filled(
                row.cell_rect(canvas, start, end.saturating_sub(start)),
                2.0,
                canvas.ui.visuals().warn_fg_color.gamma_multiply(alpha),
            );
        }
        let Some((open, close)) = marks.brackets else {
            return;
        };
        for end in [open, close] {
            if end.line == line && end.col >= visual.start && end.col < visual.end {
                canvas.painter.rect_stroke(
                    row.cell_rect(canvas, end.col, 1),
                    2.0,
                    marks.bracket_stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    /// Highlighted row text, spans clipped to this visual row.
    fn paint_row_text(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let visual = row.visual;
        let spans: Vec<Span> = canvas
            .highlight
            .line(visual.line)
            .iter()
            .filter_map(|span| {
                let start = span.start.max(visual.start);
                let end = span.end.min(visual.end);
                (start < end).then_some(Span {
                    start: start.saturating_sub(visual.start),
                    end: end.saturating_sub(visual.start),
                    color: span.color,
                    style: span.style,
                })
            })
            .collect();
        RowPaint {
            painter: &canvas.painter,
            text: &visual.text,
            hints: row.cells.hints,
            spans: &spans,
            x: canvas.origin.x + self.geometry.gutter,
            y: row.y,
            geometry: self.geometry,
            plain: canvas.ui.visuals().text_color(),
        }
        .paint();
    }

    /// `⋯` placeholder after a folded region's first line.
    fn paint_fold_placeholder(&mut self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        if !row.visual.folded {
            return;
        }
        let geometry = self.geometry;
        let visuals = canvas.ui.visuals();
        let at = row
            .cells
            .char_x(row.visual.text.chars().count())
            .saturating_add(1);
        let rect = Rect::from_min_size(
            Pos2::new(canvas.text_x(at), row.y + 1.0),
            Vec2::new(geometry.advance * 3.0, geometry.row_height - 2.0),
        );
        canvas
            .painter
            .rect_filled(rect, 3.0, visuals.faint_bg_color);
        canvas.painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "⋯",
            geometry.font.clone(),
            visuals.weak_text_color(),
        );
        let line = row.line();
        let hit = canvas.ui.interact(
            rect,
            geometry.focus_id.with(("fold-placeholder", line)),
            Sense::click(),
        );
        if hit.on_hover_text("Click to unfold").clicked() {
            self.out.fold_toggles.push(line);
        }
    }

    /// Diagnostic underlines on this row, with their message on hover.
    fn paint_row_squiggles(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let (visual, line) = (row.visual, row.line());
        let squiggles = &self.options.squiggles;
        let first = squiggles.partition_point(|squiggle| squiggle.line < line);
        for (index, squiggle) in squiggles
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(_, squiggle)| squiggle.line == line)
        {
            let (start, end) = (
                squiggle.start.max(visual.start),
                squiggle
                    .end
                    .min(visual.end.max(visual.start.saturating_add(1))),
            );
            if start >= end {
                continue;
            }
            let rect = row.cell_rect(canvas, start, end.saturating_sub(start));
            let color = RenderHelpers::severity_color(canvas.ui.visuals(), squiggle.severity);
            RenderHelpers::paint_squiggle(&canvas.painter, rect, color);
            canvas
                .ui
                .interact(
                    rect,
                    self.geometry.focus_id.with(("squiggle", index)),
                    Sense::hover(),
                )
                .on_hover_text(&squiggle.message);
        }
    }

    fn paint_extra_carets(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>, marks: &RowMarks) {
        let visual = row.visual;
        for caret in &marks.extra_carets {
            if marks.cursor_visible && visual.owns_cursor(caret.cursor.line, caret.cursor.col) {
                let cells = row
                    .cells
                    .caret_x(caret.cursor.col.saturating_sub(visual.start));
                canvas.painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(canvas.text_x(cells), row.y),
                        Vec2::new(2.0, self.geometry.row_height),
                    ),
                    0.0,
                    canvas.ui.visuals().text_color(),
                );
            }
        }
    }

    /// The primary caret: a bar between chars (Insert, modeless) or an
    /// opaque block over the char it covers.
    fn paint_caret(&mut self, canvas: &Canvas<'_>, row: &PaintRow<'_>, marks: &RowMarks) {
        let (visual, cursor) = (row.visual, marks.cursor);
        if !marks.cursor_visible || !visual.owns_cursor(cursor.line, cursor.col) {
            return;
        }
        let geometry = self.geometry;
        let visuals = canvas.ui.visuals();
        let local_col = cursor.col.saturating_sub(visual.start);
        let cell = visual.text.chars().nth(local_col);
        let cells = if marks.bar {
            row.cells.caret_x(local_col)
        } else {
            row.cells.char_x(local_col)
        };
        let x = canvas.text_x(cells);
        let at = Pos2::new(x, row.y);
        self.out.caret_rect = Some(Rect::from_min_size(
            at,
            Vec2::new(geometry.advance, geometry.row_height),
        ));
        // Modeless carets are always a bar: they sit between chars.
        if marks.bar {
            canvas.painter.rect_filled(
                Rect::from_min_size(at, Vec2::new(2.0, geometry.row_height)),
                0.0,
                visuals.text_color(),
            );
            return;
        }
        // A tab's block covers the cells up to its stop.
        let cell_cells = if cell.is_some() {
            row.cells
                .caret_x(local_col.saturating_add(1))
                .saturating_sub(cells)
        } else {
            1
        };
        let cursor_w = cell_cells as f32 * geometry.advance;
        // Opaque caret in the text color (IntelliJ's block caret), not the
        // muted selection blue: a #214283 block on the #1E1F22 surface is
        // nearly invisible.
        canvas.painter.rect_filled(
            Rect::from_min_size(at, Vec2::new(cursor_w.max(2.0), geometry.row_height)),
            0.0,
            visuals.text_color(),
        );
        // Keep the covered cell legible on the block cursor.
        if let Some(cell) = cell {
            canvas.painter.text(
                at,
                egui::Align2::LEFT_TOP,
                cell.to_string(),
                geometry.font.clone(),
                visuals.window_fill,
            );
        }
    }
}

impl Canvas<'_> {
    /// One guide per indent level the line sits inside; returns the
    /// guide width the next row inherits.
    fn paint_indent_guides(&self, row: &PaintRow<'_>, guide_cells: usize, color: Color32) -> usize {
        if row.visual.start != 0 {
            return guide_cells;
        }
        let text = &row.visual.text;
        let lead: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let guide_cells = if lead.chars().count() < text.chars().count() {
            display_x(&lead, lead.chars().count())
        } else {
            guide_cells
        };
        let y = row.y;
        for cell in (0..guide_cells).step_by(crate::rows::TAB_WIDTH) {
            let x = self.text_x(cell) + 1.0;
            self.painter.vline(
                x,
                y..=y + self.geometry.row_height,
                egui::Stroke::new(1.0, color),
            );
        }
        guide_cells
    }
}
