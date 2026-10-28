//! Painted diff rows, panes, and their shared metrics.
use super::super::*;

mod document;
mod rows;

pub(super) use document::*;
pub(super) use rows::*;

#[derive(Clone, Copy)]
pub(super) struct DiffColors {
    pub(super) added: Color32,
    pub(super) deleted: Color32,
    pub(super) accent: Color32,
    pub(super) text: Color32,
}

#[derive(Clone, Copy)]
pub(super) enum DiffGutter {
    Unified,
    Old,
    New,
}

pub(super) struct DiffMetrics {
    pub(super) font: egui::FontId,
    digit_w: f32,
    pub(super) row_h: f32,
    digits: u32,
}

impl DiffMetrics {
    pub(super) fn measure(ui: &egui::Ui, digits: u32) -> Self {
        let Some(font) = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Monospace)
            .cloned()
        else {
            return Self {
                font: egui::FontId::monospace(12.0),
                digit_w: 0.0,
                row_h: 0.0,
                digits,
            };
        };
        let (digit_w, row_h) = ui
            .ctx()
            .fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
        Self {
            font,
            digit_w,
            row_h,
            digits,
        }
    }

    pub(super) fn number_w(&self) -> f32 {
        // Gutter digit column is a UI coordinate.
        let digits = self.digits as f32;
        self.digit_w * digits
    }
}

pub(super) fn diff_row_height(ui: &egui::Ui) -> f32 {
    DiffMetrics::measure(ui, 4).row_h
}

#[derive(Clone, Copy)]
pub(super) struct GutterCols {
    pub(super) old_right: Option<f32>,
    pub(super) new_right: Option<f32>,
    pub(super) sign: f32,
    pub(super) code: f32,
}

pub(super) struct DiffPaint<'a> {
    pub(super) width: f32,
    pub(super) line: &'a diff::DiffLine,
    pub(super) gutter: DiffGutter,
    pub(super) colors: DiffColors,
    pub(super) metrics: &'a DiffMetrics,
}

pub(super) struct DiffSidePaint<'a> {
    size: egui::Vec2,
    line: Option<&'a diff::DiffLine>,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

pub(super) struct DiffGutterPaint<'a> {
    pub(super) rect: egui::Rect,
    pub(super) cols: GutterCols,
    pub(super) line: &'a diff::DiffLine,
    pub(super) sign: &'static str,
    pub(super) colors: DiffColors,
    pub(super) metrics: &'a DiffMetrics,
}

pub(super) struct DiffNumberPaint<'a> {
    top: f32,
    right: f32,
    number: Option<u32>,
    metrics: &'a DiffMetrics,
    color: Color32,
}

// Widths are document-wide so vertical virtualization cannot shrink the horizontal
// scroll range when the longest line leaves the viewport.
#[derive(Clone)]
pub(super) struct DiffWidths {
    fingerprint: egui::Id,
    content: f32,
}

pub(crate) struct DiffContentWidthInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) metrics: &'a DiffMetrics,
    pub(crate) scroll_key: &'a str,
}

pub(super) fn diff_row_count(doc: &diff::DiffDocument, split: bool) -> usize {
    if split {
        doc.split.len()
    } else {
        doc.unified.len()
    }
}

pub(super) fn diff_doc_digits(doc: &diff::DiffDocument, split: bool) -> u32 {
    if split {
        diff_gutter_digits(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        diff_gutter_digits(doc.unified.iter())
    }
}

pub(super) fn diff_gutter_digits<'a>(lines: impl IntoIterator<Item = &'a diff::DiffLine>) -> u32 {
    let max = lines
        .into_iter()
        .flat_map(|line| [line.old_no, line.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    max.ilog10().saturating_add(1).max(4)
}

pub(super) fn gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let number_w = metrics.number_w();
    let mut x = pad;
    match gutter {
        DiffGutter::Unified => {
            let old_right = x + number_w;
            x = old_right + pad;
            let new_right = x + number_w;
            x = new_right + pad;
            let sign = x;
            GutterCols {
                old_right: Some(old_right),
                new_right: Some(new_right),
                sign,
                code: sign + metrics.digit_w + pad,
            }
        }
        DiffGutter::Old | DiffGutter::New => side_gutter_cols(metrics, gutter),
    }
}

pub(super) fn side_gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let right = pad + metrics.number_w();
    let sign = right + pad;
    let code = sign + metrics.digit_w + pad;
    GutterCols {
        old_right: matches!(gutter, DiffGutter::Old).then_some(right),
        new_right: matches!(gutter, DiffGutter::New).then_some(right),
        sign,
        code,
    }
}

pub(super) fn diff_row_style(kind: diff::LineKind, colors: DiffColors) -> (Color32, &'static str) {
    match kind {
        diff::LineKind::Insert => (tint(colors.added, 40), "+"),
        diff::LineKind::Delete => (tint(colors.deleted, 40), "-"),
        diff::LineKind::Hunk => (Color32::TRANSPARENT, " "),
        diff::LineKind::Equal => (Color32::TRANSPARENT, " "),
    }
}

pub(crate) struct RowColorsInput<'a> {
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) row: usize,
    pub(crate) split: bool,
    pub(crate) colors: DiffColors,
}

pub(super) fn row_colors(input: RowColorsInput<'_>) -> DiffColors {
    let RowColorsInput {
        doc,
        row,
        split,
        colors,
    } = input;

    if doc
        .change_at(row, split)
        .is_some_and(diff::review::Hunk::replacement)
    {
        DiffColors {
            added: colors.accent,
            deleted: colors.accent,
            ..colors
        }
    } else {
        colors
    }
}

pub(super) fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
