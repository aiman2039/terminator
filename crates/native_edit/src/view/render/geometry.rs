//! Per-frame metrics: fonts, cell size, gutter layout and the viewport the
//! scroll area gets.
use super::{ANNOTATION_CELLS, SourceOptions};
use crate::doc::Buffer;
use crate::highlight::HighlightCache;
use crate::view::{BOLD_FAMILY, ITALIC_FAMILY, source_focus_id};
use egui::FontId;

/// Gap between the gutter hairline and the first text cell.
const GUTTER_GAP: f32 = 6.0;
/// Minimum height of the status bar under the source view.
const STATUS_HEIGHT: f32 = 22.0;

/// Round a coordinate to the nearest physical pixel, so glyphs, carets
/// and fills land on the pixel grid instead of blurring across two.
pub(super) fn snap(value: f32, ppp: f32) -> f32 {
    if ppp > 0.0 {
        (value * ppp).round() / ppp
    } else {
        value
    }
}

/// Row pitch and text offset for a font whose glyph row is `font_row`
/// tall: `line_height` (clamped to 1.0..=2.0) scales the pitch, which
/// rounds to whole physical pixels, and the text sits centered in it.
pub(super) fn row_metrics(font_row: f32, line_height: f32, ppp: f32) -> (f32, f32) {
    let factor = if line_height.is_finite() {
        line_height.clamp(1.0, 2.0)
    } else {
        1.0
    };
    let row_height = snap(font_row * factor, ppp)
        .max(snap(font_row, ppp))
        .max(1.0);
    let text_dy = snap(((row_height - font_row) * 0.5).max(0.0), ppp);
    (row_height, text_dy)
}

/// Row pitch [`super::show_source`] paints with `options`, for hosts that
/// lay another pane out on the same rows.
#[must_use]
pub fn source_row_height(ui: &egui::Ui, options: &SourceOptions) -> f32 {
    let font = FontId::monospace(options.font_size);
    let font_row = ui.fonts_mut(|fonts| fonts.row_height(&font).max(1.0));
    row_metrics(font_row, options.line_height, ui.ctx().pixels_per_point()).0
}

/// Fonts, cell metrics and gutter layout, measured once per frame before
/// the scroll area opens.
pub(super) struct Geometry {
    pub font: FontId,
    pub italic_font: FontId,
    pub bold_font: FontId,
    /// Width of one monospace cell.
    pub advance: f32,
    /// Painted row pitch: the font's row height times the line-spacing
    /// factor, whole physical pixels.
    pub row_height: f32,
    /// Offset from a row's top to its text, centering the glyph row.
    pub text_dy: f32,
    /// Physical pixels per point, for snapping painted positions.
    pub ppp: f32,
    /// Blame column: as wide as the widest annotation (capped), between
    /// the change band and the line numbers; 0 without annotations.
    pub annotation_w: f32,
    /// Change band, annotations, line numbers and fold arrows end here:
    /// the hairline between gutter and text.
    pub rule_x: f32,
    /// Text starts here: `rule_x` plus a small gap.
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
        let ppp = ui.ctx().pixels_per_point();
        let (row_height, text_dy) = row_metrics(font_row, options.line_height, ppp);
        let annotation_w = Self::annotation_width(options, advance);
        let (rule_x, gutter) = if options.show_line_numbers {
            let digits = doc.line_count().max(1).to_string().len() as f32;
            let rule_x = snap((digits + 2.0) * advance + annotation_w, ppp);
            (rule_x, rule_x + snap(GUTTER_GAP, ppp))
        } else {
            (0.0, 0.0)
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
            text_dy,
            ppp,
            annotation_w,
            rule_x,
            gutter,
            wrap_columns,
            footer_height: if options.show_status {
                snap(font_row.max(STATUS_HEIGHT), ppp)
            } else {
                0.0
            },
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

    /// `(scroll_fraction, scroll_offset)`: a host-set position, as a
    /// fraction of the document or in points (the offset wins).
    pub fn scroll_area(
        &self,
        id: &str,
        (scroll_fraction, scroll_offset): (Option<f32>, Option<f32>),
    ) -> egui::ScrollArea {
        let mut scroll = if self.h_scroll {
            egui::ScrollArea::both()
        } else {
            egui::ScrollArea::vertical()
        };
        scroll = scroll
            .id_salt(("native-source", id))
            .max_height(self.max_scroll)
            .auto_shrink([!self.h_scroll, false]);
        if let Some(offset) = scroll_offset {
            scroll = scroll.vertical_scroll_offset(offset.max(0.0));
        } else if let Some(fraction) = scroll_fraction {
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
