use super::super::super::*;
use super::{HighlightedJobInput, ensure_cache, highlighted_job};

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// The split diff's right pane is a live editor: no edit mode, the buffer is
/// edited in place with syntax highlighting and diff tints. Returns a
/// `ScrollAreaOutput<()>` shaped like the painted panes so hunk controls and
/// connectors keep working unchanged.
pub(crate) struct PaintRightEditorInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) rect: egui::Rect,
    pub(crate) buffer: &'a mut diff::review::Edit,
    pub(crate) path: &'a std::path::Path,
    pub(crate) colors: super::super::diff_paint::DiffColors,
    pub(crate) sync: f32,
    pub(crate) key: &'a str,
}

/// Geometry of the right editor pane: a number gutter, then the editor.
struct RightPane {
    rect: egui::Rect,
    editor_rect: egui::Rect,
    font: egui::FontId,
    row_h: f32,
    line_count: usize,
}

/// The cached highlight data the editor's layouter paints from.
struct RightText<'a> {
    spans: &'a [Vec<diff::DiffSpan>],
    valid: &'a [bool],
    tints: &'a [Option<Color32>],
    fallback: Color32,
    sync: f32,
    key: &'a str,
}

pub(in super::super) fn paint_right_editor(input: PaintRightEditorInput<'_>) -> PaneOutput {
    let PaintRightEditorInput {
        ui,
        rect,
        buffer,
        path,
        colors,
        sync,
        key,
    } = input;

    let (spans, valid, tints, _rows) = ensure_cache(buffer, path, colors);
    let pane = RightPane::measure(ui, rect, &buffer.text);
    let text = RightText {
        spans: &spans,
        valid: &valid,
        tints: &tints,
        fallback: colors.text,
        sync,
        key,
    };
    let output = pane.show_editor(ui, &text, buffer);
    // Full-width background blocks behind changed lines, on the background
    // layer so they sit under the text: the line *and* its block highlight,
    // recomputed live from the buffer on every revision.
    if pane.row_h > 0.0 {
        pane.paint_tint_blocks(ui, &output, &tints);
    }
    // Paint buffer line numbers in the reserved gutter.
    if pane.row_h > 0.0 {
        pane.paint_line_numbers(ui, &output);
    }
    egui::scroll_area::ScrollAreaOutput {
        inner: (),
        id: output.id,
        state: output.state,
        content_size: output.content_size,
        inner_rect: output.inner_rect,
    }
}

impl RightPane {
    /// Reserve a number gutter inside the pane, painted after the editor.
    fn measure(ui: &egui::Ui, rect: egui::Rect, text: &str) -> Self {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let line_count = text.split_inclusive('\n').count().max(1);
        let digits = line_count.to_string().len().max(1) as f32;
        let (digit_w, row_h) = ui
            .ctx()
            .fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
        let gutter_w = digit_w * digits + 10.0;
        let editor_rect =
            egui::Rect::from_min_max(egui::pos2(rect.left() + gutter_w, rect.top()), rect.max);
        Self {
            rect,
            editor_rect,
            font,
            row_h,
            line_count,
        }
    }

    fn show_editor(
        &self,
        ui: &mut egui::Ui,
        text: &RightText<'_>,
        buffer: &mut diff::review::Edit,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        let key = text.key;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(self.editor_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::both()
                    .id_salt((key, "diff-right-editor"))
                    .auto_shrink([false, false])
                    .vertical_scroll_offset(text.sync)
                    .show(ui, |ui| self.text_editor(ui, text, buffer))
            },
        )
        .inner
    }

    fn text_editor(
        &self,
        ui: &mut egui::Ui,
        text: &RightText<'_>,
        buffer: &mut diff::review::Edit,
    ) {
        let mut layouter = |ui: &egui::Ui, buffer_text: &dyn egui::TextBuffer, wrap_width: f32| {
            ui.ctx().fonts_mut(|fonts| {
                fonts.layout_job(highlighted_job(HighlightedJobInput {
                    text: buffer_text.as_str(),
                    spans: text.spans,
                    valid: text.valid,
                    tints: text.tints,
                    font: self.font.clone(),
                    fallback: text.fallback,
                    wrap_width,
                }))
            })
        };
        let editor = egui::TextEdit::multiline(&mut buffer.text)
            .id(egui::Id::unique((text.key, "diff-right-text")))
            .font(egui::TextStyle::Monospace)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .layouter(&mut layouter)
            .desired_width(f32::INFINITY)
            .show(ui);
        if editor.response.changed() {
            buffer.stale = true;
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-right-editor", editor.response.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = &editor;
    }

    /// First visible row and how many rows the viewport shows.
    fn visible_rows(&self, output: &PaneOutput) -> (usize, usize) {
        let row_h = self.row_h;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Row index from a non-negative scroll offset"
        )]
        let first = (output.state.unclamped_offset().y / row_h).floor().max(0.0) as usize;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::arithmetic_side_effects,
            reason = "Visible row count from the viewport height"
        )]
        let visible = (output.inner_rect.height() / row_h).ceil() as usize + 1;
        (first, visible)
    }

    fn paint_tint_blocks(&self, ui: &egui::Ui, output: &PaneOutput, tints: &[Option<Color32>]) {
        let (rect, row_h) = (self.rect, self.row_h);
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(rect);
        let (first, visible) = self.visible_rows(output);
        for (index, tint) in tints.iter().enumerate().skip(first).take(visible) {
            let Some(tint) = tint else { continue };
            let top =
                output.inner_rect.top() + index as f32 * row_h - output.state.unclamped_offset().y;
            let block = egui::Rect::from_min_max(
                egui::pos2(rect.left(), top),
                egui::pos2(rect.right(), top + row_h),
            )
            .intersect(rect);
            if block.is_positive() {
                let fill = Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 115);
                painter.rect_filled(block, 0, fill);
            }
        }
    }

    fn paint_line_numbers(&self, ui: &egui::Ui, output: &PaneOutput) {
        let row_h = self.row_h;
        let painter = ui.painter().with_clip_rect(self.rect);
        let (first, visible) = self.visible_rows(output);
        for index in first..self.line_count.min(first.saturating_add(visible)) {
            let center_y = output.inner_rect.top() + index as f32 * row_h
                - output.state.unclamped_offset().y
                + row_h / 2.0;
            painter.text(
                egui::pos2(self.editor_rect.left() - 5.0, center_y),
                egui::Align2::RIGHT_CENTER,
                index.saturating_add(1).to_string(),
                self.font.clone(),
                ui.visuals().weak_text_color(),
            );
        }
    }
}
