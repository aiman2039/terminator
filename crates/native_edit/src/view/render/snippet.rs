//! Snippet stops in the source view ([`crate::vim::VimEngine::start_snippet`]):
//! the keys a session owns, and the stop outlines.
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use super::helpers::{RenderHelpers, SelectionSpanForLineInput};
use crate::doc::Buffer;
use crate::vim::{Key as VKey, SnippetMark};

impl<B: Buffer> SourceView<'_, B> {
    /// A key the snippet session owns. Tab and Shift+Tab move between
    /// stops and Enter finishes. Escape ends the session and keeps its
    /// usual meaning. In vim, Backspace and Delete remove a stop that
    /// still reads as selected. True when the key was used up.
    pub(super) fn snippet_key(&mut self, vkey: VKey) -> bool {
        if !self.engine.snippet_active() {
            return false;
        }
        match vkey {
            VKey::Tab | VKey::BackTab => self.engine.snippet_jump(self.doc, vkey == VKey::Tab),
            VKey::Enter => self.engine.finish_snippet(self.doc),
            VKey::Backspace | VKey::Delete => self.engine.snippet_take_selection(self.doc),
            VKey::Escape => {
                self.engine.end_snippet();
                false
            }
            _ => false,
        }
    }

    /// Outline the stops on this row: the copies of the selected stop in
    /// the selection color (filled while vim shows them as selected),
    /// the other stops faintly.
    pub(super) fn paint_snippet_marks(
        &self,
        canvas: &Canvas<'_>,
        row: &PaintRow<'_>,
        marks: &[SnippetMark],
    ) {
        let visuals = canvas.ui.visuals();
        let (line, line_len) = (row.line(), row.visual.line_len);
        for mark in marks {
            let span = RenderHelpers::selection_span_for_line(SelectionSpanForLineInput {
                a: mark.start,
                b: mark.end,
                line,
                line_len,
                inclusive: false,
            });
            let Some(rect) = span.and_then(|(start, end)| row.span_rect(canvas, start, end)) else {
                continue;
            };
            if mark.selected {
                canvas
                    .painter
                    .rect_filled(rect, 2.0, visuals.selection.bg_fill);
            }
            let color = if mark.active {
                visuals.selection.stroke.color
            } else {
                visuals.weak_text_color().gamma_multiply(0.6)
            };
            canvas.painter.rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
        }
    }
}
