//! Per-row gutter: change band, diagnostic strip, fold arrow, blame
//! annotation and line number.
use super::canvas::{Canvas, PaintRow};
use super::geometry::snap;
use super::helpers::RenderHelpers;
use super::palette::EditorPalette;
use super::{ANNOTATION_CELLS, CHANGE_BAND, ChangeKind, SourceView};
use crate::doc::Buffer;
use crate::lsp::Severity;
use egui::{Pos2, Rect, Sense, Vec2};

impl<B: Buffer> SourceView<'_, B> {
    /// The gutter's tinted band and the 1px hairline between it and the
    /// text, over the visible height.
    pub(super) fn paint_gutter_band(&self, canvas: &Canvas<'_>, palette: &EditorPalette) {
        if !self.options.show_line_numbers {
            return;
        }
        let (origin, clip) = (canvas.origin, canvas.clip);
        let rule_x = origin.x + self.geometry.rule_x;
        canvas.painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(origin.x, clip.top()),
                Pos2::new(rule_x, clip.bottom()),
            ),
            0.0,
            palette.gutter_fill,
        );
        // Centered on one physical pixel, so the hairline stays 1px sharp.
        let ppp = self.geometry.ppp;
        canvas.painter.vline(
            snap(rule_x, ppp) - 0.5 / ppp,
            clip.top()..=clip.bottom(),
            egui::Stroke::new(1.0 / ppp, palette.gutter_rule),
        );
    }

    /// Gutter cells of a row's first visual line; `number_color` is the
    /// line number's color unless a diagnostic tints it.
    pub(super) fn paint_gutter(
        &mut self,
        canvas: &Canvas<'_>,
        row: &PaintRow<'_>,
        number_color: egui::Color32,
    ) {
        if row.visual.start != 0 {
            return;
        }
        if self.options.show_line_numbers {
            self.paint_change_band(canvas, row);
        }
        // Worst severity on the row, for both the line-number tint and
        // the gutter strip.
        let severity = self.row_severity(row.line());
        if let Some(severity) = severity {
            // A 2px strip at the pane edge survives a scan of the line
            // numbers, where a color alone does not.
            canvas.painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(canvas.origin.x, row.y),
                    Vec2::new(2.0, self.geometry.row_height),
                ),
                0.0,
                RenderHelpers::severity_color(canvas.ui.visuals(), severity),
            );
        }
        self.paint_fold_arrow(canvas, row);
        self.paint_annotation(canvas, row);
        let number_color = severity.map_or(number_color, |severity| {
            RenderHelpers::severity_color(canvas.ui.visuals(), severity)
        });
        self.paint_line_number(canvas, row, number_color);
    }

    fn row_severity(&self, line: usize) -> Option<Severity> {
        let diagnostics = &self.options.diagnostics;
        diagnostics
            .binary_search_by(|mark| mark.line.cmp(&line))
            .ok()
            .and_then(|at| diagnostics.get(at))
            .map(|mark| mark.severity)
    }

    /// VCS change band: a bar beside changed lines, a wedge where lines
    /// were deleted.
    fn paint_change_band(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let line = row.line();
        let marks = &self.options.change_marks;
        let at = marks.partition_point(|mark| mark.end.max(mark.start.saturating_add(1)) <= line);
        let Some(mark) = marks.get(at).filter(|mark| mark.start <= line) else {
            return;
        };
        let [added, modified, deleted] = self.options.change_colors;
        let (origin, y) = (canvas.origin, row.y);
        match mark.kind {
            ChangeKind::Added | ChangeKind::Modified => {
                canvas.painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x + 3.0, y),
                        Vec2::new(3.0, self.geometry.row_height),
                    ),
                    0.0,
                    if mark.kind == ChangeKind::Added {
                        added
                    } else {
                        modified
                    },
                );
            }
            ChangeKind::Deleted => {
                canvas.painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(origin.x + 2.0, y - 3.0),
                        Pos2::new(origin.x + 7.0, y),
                        Pos2::new(origin.x + 2.0, y + 3.0),
                    ],
                    deleted,
                    egui::Stroke::NONE,
                ));
            }
        }
    }

    /// Fold arrow in the gutter's last cell: always on folded blocks,
    /// on open ones while the pointer is in the gutter.
    fn paint_fold_arrow(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let line = row.line();
        if !self.options.show_line_numbers || canvas.folds.region_at(line).is_none() {
            return;
        }
        let (origin, gutter) = (canvas.origin, self.geometry.rule_x);
        let folded = canvas.folds.is_folded(line);
        let pointer_in_gutter = canvas
            .ui
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|pos| {
                canvas.response.rect.contains(pos) && pos.x < origin.x + self.geometry.gutter
            });
        if !folded && !pointer_in_gutter {
            return;
        }
        let advance = self.geometry.advance;
        let center = Pos2::new(
            origin.x + gutter - advance * 0.5,
            row.y + self.geometry.row_height * 0.5,
        );
        let size = (advance * 0.35).max(3.0);
        let points = if folded {
            vec![
                Pos2::new(center.x - size * 0.5, center.y - size),
                Pos2::new(center.x + size * 0.7, center.y),
                Pos2::new(center.x - size * 0.5, center.y + size),
            ]
        } else {
            vec![
                Pos2::new(center.x - size, center.y - size * 0.5),
                Pos2::new(center.x + size, center.y - size * 0.5),
                Pos2::new(center.x, center.y + size * 0.7),
            ]
        };
        canvas.painter.add(egui::Shape::convex_polygon(
            points,
            canvas.ui.visuals().weak_text_color(),
            egui::Stroke::NONE,
        ));
    }

    /// Blame cell: commit text, alternating shade per commit, hover text
    /// and a click target.
    fn paint_annotation(&mut self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let line = row.line();
        let geometry = self.geometry;
        let Some(annotation) = self
            .options
            .annotations
            .as_ref()
            .filter(|_| geometry.annotation_w > 0.0)
            .and_then(|annotations| annotations.get(line))
        else {
            return;
        };
        let visuals = canvas.ui.visuals();
        let cell = Rect::from_min_size(
            Pos2::new(canvas.origin.x + CHANGE_BAND, row.y),
            Vec2::new(
                geometry.annotation_w - geometry.advance * 0.5,
                geometry.row_height,
            ),
        );
        if annotation.group % 2 == 1 {
            canvas
                .painter
                .rect_filled(cell, 0.0, visuals.faint_bg_color);
        }
        let text: String = annotation.text.chars().take(ANNOTATION_CELLS).collect();
        canvas.painter.text(
            Pos2::new(cell.left(), cell.top() + geometry.text_dy),
            egui::Align2::LEFT_TOP,
            text,
            geometry.font.clone(),
            visuals.weak_text_color(),
        );
        if annotation.hover.is_empty() {
            return;
        }
        let hit = canvas
            .ui
            .interact(
                cell,
                geometry.focus_id.with(("annotation", line)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&annotation.hover);
        if hit.clicked() {
            self.out.annotation_clicked = Some(line);
        }
    }

    fn paint_line_number(
        &self,
        canvas: &Canvas<'_>,
        row: &PaintRow<'_>,
        number_color: egui::Color32,
    ) {
        if !self.options.show_line_numbers {
            return;
        }
        let geometry = self.geometry;
        canvas.painter.text(
            Pos2::new(
                snap(
                    canvas.origin.x + geometry.rule_x - geometry.advance,
                    geometry.ppp,
                ),
                row.y + geometry.text_dy,
            ),
            egui::Align2::RIGHT_TOP,
            format!("{}", row.line().saturating_add(1)),
            geometry.font.clone(),
            number_color,
        );
    }
}
