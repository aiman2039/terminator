//! Text painting for one row: highlight spans, plain gaps and inlay hints.
use super::geometry::{Geometry, snap};
use crate::highlight::{Span, SpanStyle};
use crate::rows::{RowCells, RowHint, advance_cell, expand_tabs};
use egui::{Color32, FontId, Pos2, Rect, Vec2};

/// One run of row text in a single style, starting at char `col`.
struct PieceInput<'a> {
    col: usize,
    piece: &'a str,
    font: &'a FontId,
    color: Color32,
}

/// Paint one row: plain in a single shape, or one shape per highlight
/// span with a plain tail for anything the spans do not cover. Piece
/// positions use display cells, so wide chars keep every color aligned.
pub(super) struct RowPaint<'a> {
    pub painter: &'a egui::Painter,
    pub text: &'a str,
    /// Inlay hints: painted as weak boxed text, pushing the chars after
    /// them right (see [`RowCells`]).
    pub hints: &'a [RowHint<'a>],
    pub spans: &'a [Span],
    pub x: f32,
    pub y: f32,
    /// Fonts and cell advance.
    pub geometry: &'a Geometry,
    pub plain: Color32,
    /// The syntax theme's default foreground; spans in it paint `plain`.
    pub theme_plain: Option<[u8; 3]>,
}

impl RowPaint<'_> {
    /// Font for one span: theme emphasis picks the italic/bold family.
    fn font_for(&self, style: SpanStyle) -> FontId {
        if style.italic {
            self.geometry.italic_font.clone()
        } else if style.bold {
            self.geometry.bold_font.clone()
        } else {
            self.geometry.font.clone()
        }
    }

    /// Paint `piece` (row text from char `col`, display cell `cell`) in one
    /// style, split at every hint inside it so the hint gaps open between
    /// chars. Returns the display cell after the piece, so the caller walks
    /// the row once instead of rescanning it from column 0 for each span.
    /// Parts outside the painter's clip rect are skipped.
    fn piece(&self, input: PieceInput<'_>, cell: usize) -> usize {
        let PieceInput {
            col,
            piece,
            font,
            color,
        } = input;
        let tab = self.geometry.tab;
        let geometry = RowCells {
            text: self.text,
            hints: self.hints,
            tab,
        };
        let len = piece.chars().count();
        let end = col.saturating_add(len);
        let mut breaks: Vec<usize> = self
            .hints
            .iter()
            .map(|hint| hint.col)
            .filter(|at| *at > col && *at < end)
            .collect();
        breaks.dedup();
        breaks.push(end);
        let clip = self.painter.clip_rect();
        let advance = self.geometry.advance;
        let mut chars = piece.chars();
        let mut from = col;
        let mut cell = cell;
        for to in breaks {
            let part: String = chars.by_ref().take(to.saturating_sub(from)).collect();
            let next = part.chars().fold(cell, |at, c| advance_cell(at, c, tab));
            if !part.is_empty() {
                let shift = geometry.hint_shift(from);
                let x = self.x + cell.saturating_add(shift) as f32 * advance;
                let right = self.x + next.saturating_add(shift) as f32 * advance;
                if right >= clip.left() && x <= clip.right() {
                    let ppp = self.geometry.ppp;
                    self.painter.text(
                        Pos2::new(snap(x, ppp), snap(self.y, ppp)),
                        egui::Align2::LEFT_TOP,
                        expand_tabs(&part, cell, tab),
                        font.clone(),
                        color,
                    );
                }
            }
            from = to;
            cell = next;
        }
        cell
    }

    pub fn paint(&self) {
        self.paint_hints();
        if self.spans.is_empty() {
            self.piece(
                PieceInput {
                    col: 0,
                    piece: self.text,
                    font: &self.geometry.font,
                    color: self.plain,
                },
                0,
            );
            return;
        }
        let mut chars = self.text.chars();
        let mut col = 0usize;
        let mut cell = 0usize;
        for span in self.spans {
            // Unstyled gaps between spans paint plain: spans need not
            // partition the row (tree-sitter emits styled ranges only).
            let gap: String = chars
                .by_ref()
                .take(span.start.saturating_sub(col))
                .collect();
            if !gap.is_empty() {
                cell = self.piece(
                    PieceInput {
                        col,
                        piece: &gap,
                        font: &self.geometry.font,
                        color: self.plain,
                    },
                    cell,
                );
                col = span.start;
            }
            let piece: String = chars.by_ref().take(span.end.saturating_sub(col)).collect();
            if !piece.is_empty() {
                let [red, green, blue] = span.color;
                let color = if self.theme_plain == Some(span.color) {
                    self.plain
                } else {
                    Color32::from_rgb(red, green, blue)
                };
                cell = self.piece(
                    PieceInput {
                        col,
                        piece: &piece,
                        font: &self.font_for(span.style),
                        color,
                    },
                    cell,
                );
            }
            col = span.end;
        }
        let tail: String = chars.collect();
        if !tail.is_empty() {
            self.piece(
                PieceInput {
                    col,
                    piece: &tail,
                    font: &self.geometry.font,
                    color: self.plain,
                },
                cell,
            );
        }
    }

    /// Hint labels in weak text on a faint box, in the gap before their
    /// char.
    fn paint_hints(&self) {
        let geometry = RowCells {
            text: self.text,
            hints: self.hints,
            tab: self.geometry.tab,
        };
        let advance = self.geometry.advance;
        let mut seen_at: Option<(usize, usize)> = None;
        for hint in self.hints {
            // Several hints at one column sit side by side.
            let offset = match seen_at {
                Some((at, cells)) if at == hint.col => cells,
                _ => 0,
            };
            seen_at = Some((hint.col, offset.saturating_add(hint.cells)));
            let cell = geometry.caret_x(hint.col).saturating_add(offset);
            let rect = Rect::from_min_size(
                Pos2::new(
                    snap(self.x + cell as f32 * advance, self.geometry.ppp),
                    self.y + 1.0,
                ),
                Vec2::new(hint.cells as f32 * advance, self.geometry.font.size + 2.0),
            );
            let label = hint.label.trim();
            let lead = hint
                .label
                .len()
                .saturating_sub(hint.label.trim_start().len());
            let box_rect = Rect::from_min_size(
                Pos2::new(rect.left() + lead as f32 * advance, rect.top()),
                Vec2::new(
                    label
                        .chars()
                        .map(crate::rows::char_display_width)
                        .sum::<usize>() as f32
                        * advance,
                    rect.height(),
                ),
            );
            self.painter
                .rect_filled(box_rect, 3.0, self.plain.gamma_multiply(0.08));
            self.painter.text(
                box_rect.left_top(),
                egui::Align2::LEFT_TOP,
                label,
                self.geometry.font.clone(),
                self.plain.gamma_multiply(0.5),
            );
        }
    }
}
