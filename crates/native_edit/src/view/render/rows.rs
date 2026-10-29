//! Row painting: indent guides, selections, match highlights, text, fold
//! placeholders, diagnostic underlines and carets.
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use super::geometry::snap;
use super::helpers::{RenderHelpers, SelectionSpanForLineInput};
use super::palette::EditorPalette;
use super::row_paint::RowPaint;
use crate::doc::Buffer;
use crate::highlight::Span;
use crate::rows::{display_x, row_hints};
use crate::vim::ModalEngine;
use crate::vim::{Caret, Cursor, Mode};
use egui::{Color32, Pos2, Rect, Sense, Vec2};

/// Corner radius of selection and match fills.
const SELECTION_RADIUS: f32 = 2.0;

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
    /// Blink phase: `false` while the caret is in its hidden half.
    caret_on: bool,
    /// Fill the caret's row: only without a selection, like `IntelliJ`.
    current_line: Option<usize>,
    palette: EditorPalette,
    /// The syntax theme's default foreground: spans in it paint in the
    /// app's text color, so editor and chrome text match.
    theme_plain: Option<[u8; 3]>,
    /// Occurrences of the identifier at the caret, and the bracket pair
    /// around it: both paint as quiet backgrounds/outlines.
    caret_word: Option<String>,
    /// Language-server occurrences of the symbol at the caret; they
    /// replace `caret_word` while one of them holds the caret.
    symbol_ranges: Option<crate::view::SymbolRanges>,
    brackets: Option<(Cursor, Cursor)>,
    occurrence_fill: Color32,
    bracket_stroke: egui::Stroke,
    guide_color: Color32,
    /// A bar caret sits before a hint; a block covers the char after it.
    bar: bool,
}

impl<B: Buffer> SourceView<'_, B> {
    pub(super) fn row_marks(&self, canvas: &Canvas<'_>, focused: bool) -> RowMarks {
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
        let symbol_ranges = caret_word.as_ref().and_then(|_| {
            crate::view::caret_symbol_highlights(canvas.ui.ctx(), self.id, engine.cursor())
        });
        let caret_word = caret_word.filter(|_| symbol_ranges.is_none());
        let has_selection = selection.is_some()
            || engine.block_span().is_some()
            || engine
                .extra_carets()
                .iter()
                .any(|caret| caret.selection().is_some());
        RowMarks {
            selection,
            block: engine.block_span(),
            extra_carets: engine.extra_carets().to_vec(),
            cursor: engine.cursor(),
            cursor_visible,
            caret_on: self.caret_on(canvas.ui, focused),
            current_line: (cursor_visible && !has_selection).then(|| engine.cursor().line),
            palette: self.palette(visuals),
            theme_plain: crate::highlight::theme(visuals.dark_mode)
                .and_then(|theme| theme.settings.foreground)
                .map(|color| [color.r, color.g, color.b]),
            caret_word,
            symbol_ranges,
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
        self.paint_gutter_band(canvas, &marks.palette);
        for (visual_line, visual) in canvas.visible_rows() {
            let hints = row_hints(canvas.inlay_hints, visual);
            let y = canvas.origin.y + visual_line as f32 * row_height;
            let row = PaintRow::new(visual, &hints, y);
            if y + row_height < canvas.clip.top() || y > canvas.clip.bottom() {
                continue;
            }
            if visual.spacer {
                self.paint_spacer(canvas, y);
                continue;
            }
            if marks.current_line == Some(visual.line) {
                canvas.painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(canvas.origin.x + self.geometry.rule_x + 1.0, y),
                        Pos2::new(canvas.clip.right(), y + row_height),
                    ),
                    0.0,
                    marks.palette.current_line,
                );
            }
            let number_color = if marks.current_line == Some(visual.line) {
                marks.palette.current_line_number
            } else {
                marks.palette.line_number
            };
            self.paint_gutter(canvas, &row, number_color);
            self.paint_fills(canvas, &row);
            guide_cells = canvas.paint_indent_guides(&row, guide_cells, marks.guide_color);
            self.paint_selections(canvas, &row, marks);
            self.paint_match_marks(canvas, &row, marks);
            self.paint_row_text(canvas, &row, marks);
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
            canvas.painter.rect_filled(rect, SELECTION_RADIUS, fill);
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
                canvas.painter.rect_filled(rect, SELECTION_RADIUS, fill);
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
        let symbols = marks.symbol_ranges.iter().flat_map(|ranges| ranges.iter());
        for (_, start, end) in symbols.filter(|(at, _, _)| *at == line) {
            let (start, end) = ((*start).max(visual.start), (*end).min(visual.end));
            if start < end {
                let rect = row.cell_rect(canvas, start, end.saturating_sub(start));
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
            let fill = if self.options.find_current == Some(index) {
                marks.palette.find_current
            } else {
                marks.palette.find_match
            };
            canvas.painter.rect_filled(
                row.cell_rect(canvas, start, end.saturating_sub(start)),
                SELECTION_RADIUS,
                fill,
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
    fn paint_row_text(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>, marks: &RowMarks) {
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
            y: row.y + self.geometry.text_dy,
            geometry: self.geometry,
            plain: canvas.ui.visuals().text_color(),
            theme_plain: marks.theme_plain,
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
            if marks.cursor_visible
                && marks.caret_on
                && visual.owns_cursor(caret.cursor.line, caret.cursor.col)
            {
                let cells = row
                    .cells
                    .caret_x(caret.cursor.col.saturating_sub(visual.start));
                canvas.painter.rect_filled(
                    self.bar_rect(canvas.text_x(cells), row.y),
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
        if !marks.caret_on {
            return;
        }
        // Modeless carets are always a bar: they sit between chars.
        if marks.bar {
            canvas
                .painter
                .rect_filled(self.bar_rect(x, row.y), 0.0, visuals.text_color());
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
        let cursor_w = canvas.text_x(cells.saturating_add(cell_cells)) - x;
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
                Pos2::new(x, row.y + geometry.text_dy),
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
        let ppp = self.geometry.ppp;
        for cell in (0..guide_cells).step_by(crate::rows::TAB_WIDTH) {
            // Centered on a physical pixel so the 1px guide stays sharp.
            let x = snap(self.text_x(cell) + 1.0, ppp) + 0.5 / ppp;
            self.painter.vline(
                x,
                y..=y + self.geometry.row_height,
                egui::Stroke::new(1.0 / ppp, color),
            );
        }
        guide_cells
    }
}

impl<B: Buffer> SourceView<'_, B> {
    /// A 2px bar caret at `x`, whole physical pixels wide.
    fn bar_rect(&self, x: f32, y: f32) -> Rect {
        let ppp = self.geometry.ppp;
        Rect::from_min_size(
            Pos2::new(x, y),
            Vec2::new(snap(2.0, ppp).max(1.0 / ppp), self.geometry.row_height),
        )
    }
}
