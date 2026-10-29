//! Per-frame metrics: fonts, cell size, gutter layout and the viewport the
//! scroll area gets.
use super::{ANNOTATION_CELLS, SourceOptions};
use crate::doc::Buffer;
use crate::highlight::HighlightCache;
use crate::view::{BOLD_FAMILY, ITALIC_FAMILY, source_focus_id};
use egui::FontId;

/// Fonts, cell metrics and gutter layout, measured once per frame before
/// the scroll area opens.
pub(super) struct Geometry {
    pub font: FontId,
    pub italic_font: FontId,
    pub bold_font: FontId,
    /// Width of one monospace cell.
    pub advance: f32,
    pub row_height: f32,
    /// Gap above the glyphs inside a row taller than the font.
    pub text_top: f32,
    /// Blame column: as wide as the widest annotation (capped), between
    /// the change band and the line numbers; 0 without annotations.
    pub annotation_w: f32,
    /// Change band, annotations, line numbers and fold arrows.
    pub gutter: f32,
    /// Soft-wrap width in cells, `None` when long lines pan instead.
    pub wrap_columns: Option<usize>,
    /// One status row outside the source viewport (0 without status).
    pub footer_height: f32,
    /// Stable keyboard target of this view.
    pub focus_id: egui::Id,
    /// Set while a drag selection owns the pointer: the next plain click
    /// collapses it instead of extending it like a `v` selection would.
    pub mouse_drag_key: egui::Id,
}

pub struct MeasureInput<'a, B> {
    pub ui: &'a egui::Ui,
    pub doc: &'a B,
    pub options: &'a SourceOptions,
    pub id: &'a str,
}

impl Geometry {
    pub fn measure<B: Buffer>(input: MeasureInput<'_, B>) -> Self {
        let MeasureInput {
            ui,
            doc,
            options,
            id,
        } = input;
        let font = FontId::monospace(options.font_size);
        let italic_font = FontId::new(
            options.font_size,
            egui::FontFamily::Name(ITALIC_FAMILY.into()),
        );
        let bold_font = FontId::new(
            options.font_size,
            egui::FontFamily::Name(BOLD_FAMILY.into()),
        );
        let (advance, font_row) = ui.fonts_mut(|fonts| {
            (
                fonts.glyph_width(&font, 'm').max(1.0),
                fonts.row_height(&font).max(1.0),
            )
        });
        // Spacing 1.0 keeps the font's exact row height (code views).
        let row_height = if options.line_spacing > 1.0 {
            (font_row * options.line_spacing).round().max(font_row)
        } else {
            font_row
        };
        let text_top = ((row_height - font_row) * 0.5).floor().max(0.0);
        let annotation_w = Self::annotation_width(options, advance);
        let gutter = if options.show_line_numbers {
            let digits = doc.line_count().max(1).to_string().len() as f32;
            (digits + 2.0) * advance + annotation_w
        } else {
            0.0
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Pane geometry becomes a bounded number of monospace cells."
        )]
        let wrap_columns = options.soft_wrap.then(|| {
            ((ui.available_width() - gutter - 16.0).max(advance) / advance)
                .floor()
                .max(1.0) as usize
        });
        let focus_id = source_focus_id(id);
        Self {
            font,
            italic_font,
            bold_font,
            advance,
            row_height,
            text_top,
            annotation_w,
            gutter,
            wrap_columns,
            footer_height: if options.show_status { row_height } else { 0.0 },
            focus_id,
            mouse_drag_key: focus_id.with("native-source-drag"),
        }
    }

    fn annotation_width(options: &SourceOptions, advance: f32) -> f32 {
        match &options.annotations {
            Some(annotations) if options.show_line_numbers => {
                let cells = annotations
                    .iter()
                    .map(|annotation| annotation.text.chars().count())
                    .max()
                    .unwrap_or(0)
                    .clamp(8, ANNOTATION_CELLS);
                cells.saturating_add(1) as f32 * advance
            }
            _ => 0.0,
        }
    }
}

/// Scroll-area sizing, from this frame's row layout.
pub(super) struct Viewport {
    pub row_count: usize,
    pub content_height: f32,
    /// Keep one footer row outside the source viewport. The viewport must
    /// fill its allotted height even when the file has only a few lines;
    /// otherwise the command/search prompt follows the end of the file.
    pub max_scroll: f32,
    /// Long lines pan instead of clipping: without soft wrap the content
    /// keeps its full width and the scroll area owns both axes.
    pub h_scroll: bool,
    pub content_width: f32,
}

impl Viewport {
    pub fn measure(ui: &egui::Ui, geometry: &Geometry, highlight: &HighlightCache) -> Self {
        let row_count = highlight.rows().len();
        let content_height = row_count.max(1) as f32 * geometry.row_height;
        let max_scroll =
            (ui.available_height() - geometry.footer_height - ui.spacing().item_spacing.y)
                .max(geometry.row_height);
        let content_width =
            (geometry.gutter + highlight.max_cells() as f32 * geometry.advance + geometry.advance)
                .max(ui.available_width());
        Self {
            row_count,
            content_height,
            max_scroll,
            h_scroll: geometry.wrap_columns.is_none(),
            content_width,
        }
    }

    pub fn scroll_area(&self, id: &str, scroll_fraction: Option<f32>) -> egui::ScrollArea {
        let mut scroll = if self.h_scroll {
            egui::ScrollArea::both()
        } else {
            egui::ScrollArea::vertical()
        };
        scroll = scroll
            .id_salt(("native-source", id))
            .max_height(self.max_scroll)
            .auto_shrink([!self.h_scroll, false]);
        if let Some(fraction) = scroll_fraction {
            scroll = scroll.vertical_scroll_offset(
                (self.content_height - self.max_scroll).max(0.0) * fraction.clamp(0.0, 1.0),
            );
        }
        scroll
    }

    /// Current vertical position in the scrollable document, from 0 to 1.
    pub fn scroll_fraction<R>(output: &egui::scroll_area::ScrollAreaOutput<R>) -> f32 {
        let scroll_range = (output.content_size.y - output.inner_rect.height()).max(0.0);
        if scroll_range > 0.0 {
            (output.state.unclamped_offset().y / scroll_range).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}
