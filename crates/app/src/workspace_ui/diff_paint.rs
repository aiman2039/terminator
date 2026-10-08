use super::super::*;
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
    font: egui::FontId,
    digit_w: f32,
    row_h: f32,
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
        #[allow(clippy::cast_precision_loss)]
        let digits = self.digits as f32;
        self.digit_w * digits
    }
}

#[derive(Clone, Copy)]
pub(super) struct GutterCols {
    old_right: Option<f32>,
    new_right: Option<f32>,
    sign: f32,
    code: f32,
}

pub(super) struct DiffPaint<'a> {
    width: f32,
    line: &'a diff::DiffLine,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

pub(super) struct DiffSidePaint<'a> {
    size: egui::Vec2,
    line: Option<&'a diff::DiffLine>,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

pub(super) struct DiffGutterPaint<'a> {
    rect: egui::Rect,
    cols: GutterCols,
    line: &'a diff::DiffLine,
    sign: &'static str,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
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

pub(super) fn diff_content_width(
    ui: &egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    metrics: &DiffMetrics,
    scroll_key: &str,
) -> f32 {
    let cache_id = egui::Id::new(("diff-width", scroll_key, split));
    let fingerprint = egui::Id::new((
        doc,
        &metrics.font,
        metrics.digit_w.to_bits(),
        metrics.row_h.to_bits(),
        ui.ctx().pixels_per_point().to_bits(),
    ));
    if let Some(cached) = ui
        .ctx()
        .data_mut(|data| data.get_temp::<DiffWidths>(cache_id))
        && cached.fingerprint == fingerprint
    {
        return cached.content;
    }
    let lines: Box<dyn Iterator<Item = &diff::DiffLine>> = if split {
        Box::new(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        Box::new(doc.unified.iter())
    };
    let gutter = if split {
        DiffGutter::Old
    } else {
        DiffGutter::Unified
    };
    let code_width = lines
        .map(|line| {
            let text = line
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>();
            ui.painter()
                .layout_no_wrap(text, metrics.font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let content = gutter_cols(metrics, gutter).code + code_width;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            cache_id,
            DiffWidths {
                fingerprint,
                content,
            },
        )
    });
    content
}

pub(super) fn paint_diff_document(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    colors: DiffColors,
    scroll_key: &str,
    ratio: &mut f32,
    sync: &mut f32,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let rows = diff_row_count(doc, split);
    let digits = diff_doc_digits(doc, split);
    let metrics = DiffMetrics::measure(ui, digits);
    let content = diff_content_width(ui, doc, split, &metrics, scroll_key);
    if split {
        paint_diff_split(
            ui, doc, colors, &metrics, content, rows, scroll_key, ratio, sync,
        )
    } else {
        let width = content.max(ui.available_width());
        let output = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::both()
                    .id_salt((scroll_key, false))
                    .auto_shrink([false, false])
                    .show_rows(ui, metrics.row_h, rows, |ui, range| {
                        ui.set_min_width(width);
                        for index in range {
                            let Some(line) = doc.unified.get(index) else {
                                continue;
                            };
                            paint_diff_line(
                                ui,
                                DiffPaint {
                                    width,
                                    line,
                                    gutter: DiffGutter::Unified,
                                    colors,
                                    metrics: &metrics,
                                },
                            );
                        }
                    })
            })
            .inner;
        vec![output]
    }
}

pub(super) const DIFF_SPLIT_GAP: f32 = 6.0;
pub(super) const DIFF_SPLIT_MIN_PANE: f32 = 40.0;

#[derive(Clone, Copy)]
pub(super) enum SplitPane {
    Left,
    Right,
}

// Each side owns its horizontal scroll so the divider can resize them without
// clipping long lines; the vertical offset is mirrored so both sides stay in step.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_diff_split(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    colors: DiffColors,
    metrics: &DiffMetrics,
    content: f32,
    rows: usize,
    scroll_key: &str,
    ratio: &mut f32,
    sync: &mut f32,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let full = ui.available_rect_before_wrap();
    let usable = (full.width() - DIFF_SPLIT_GAP).max(1.0);
    let min_pane = DIFF_SPLIT_MIN_PANE.min(usable / 2.0);
    let left_width = (*ratio * usable).clamp(min_pane, usable - min_pane);
    let left_rect = egui::Rect::from_min_size(full.min, egui::vec2(left_width, full.height()));
    let gap_rect = egui::Rect::from_min_size(
        egui::pos2(full.left() + left_width, full.top()),
        egui::vec2(DIFF_SPLIT_GAP, full.height()),
    );
    let right_rect = egui::Rect::from_min_max(egui::pos2(gap_rect.right(), full.top()), full.max);
    // Paint both panes at the same frame-start offset so corresponding lines
    // stay aligned; whichever pane the user scrolled becomes next frame's shared
    // offset.
    let frame_sync = *sync;
    let left = paint_diff_pane(
        ui,
        left_rect,
        doc,
        SplitPane::Left,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        frame_sync,
    );
    let right = paint_diff_pane(
        ui,
        right_rect,
        doc,
        SplitPane::Right,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        frame_sync,
    );
    // Adopt the pane the pointer is over so sub-pixel input accumulates there
    // and an inactive pane's clamp cannot override the active pane. With the
    // pointer elsewhere (keyboard, divider drag), fall back to whichever pane
    // moved further from the frame-start offset.
    let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
    let over_left = pointer.is_some_and(|pos| left_rect.contains(pos));
    let over_right = pointer.is_some_and(|pos| right_rect.contains(pos));
    let left_offset = left.state.offset.y;
    let right_offset = right.state.offset.y;
    *sync = match (over_left, over_right) {
        (true, false) => left_offset,
        (false, true) => right_offset,
        _ => {
            if (left_offset - frame_sync).abs() >= (right_offset - frame_sync).abs() {
                left_offset
            } else {
                right_offset
            }
        }
    };
    ui.allocate_rect(full, egui::Sense::hover());
    let response = ui.interact(
        gap_rect,
        ui.id().with(("diff-split-drag", scroll_key)),
        egui::Sense::drag(),
    );
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        *ratio = ((pos.x - full.left()) / usable).clamp(min_pane / usable, 1.0 - min_pane / usable);
    }
    let stroke = if response.hovered() || response.dragged() {
        egui::Stroke::new(1.5, colors.accent)
    } else {
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    ui.painter()
        .vline(gap_rect.center().x, full.y_range(), stroke);
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "diff-split-handle", gap_rect);
    vec![left, right]
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_diff_pane(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &diff::DiffDocument,
    pane: SplitPane,
    colors: DiffColors,
    metrics: &DiffMetrics,
    content: f32,
    rows: usize,
    scroll_key: &str,
    sync: f32,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let side = u8::from(matches!(pane, SplitPane::Left));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            egui::ScrollArea::both()
                .id_salt((scroll_key, "split", side))
                .auto_shrink([false, false])
                .vertical_scroll_offset(sync)
                .show_rows(ui, metrics.row_h, rows, |ui, range| {
                    ui.set_min_width(content);
                    for index in range {
                        paint_diff_side(
                            ui,
                            DiffSidePaint {
                                size: egui::vec2(content, metrics.row_h),
                                line: doc.split.get(index).and_then(|row| match pane {
                                    SplitPane::Left => row.left.as_ref(),
                                    SplitPane::Right => row.right.as_ref(),
                                }),
                                gutter: match pane {
                                    SplitPane::Left => DiffGutter::Old,
                                    SplitPane::Right => DiffGutter::New,
                                },
                                colors,
                                metrics,
                            },
                        );
                    }
                })
        },
    )
    .inner
}
pub(super) fn paint_markdown_diff_preview(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    previews: &mut markdown::Previews,
    path: &std::path::Path,
    scroll_key: &str,
) -> Option<markdown::Link> {
    let mut link = None;
    ui.columns(2, |columns| {
        for (index, (col, (text, label))) in columns
            .iter_mut()
            .zip([
                (&doc.left_text, &doc.left_label),
                (&doc.right_text, &doc.right_label),
            ])
            .enumerate()
        {
            col.vertical(|ui| {
                ui.strong(label.as_str());
                let key = format!("diff-preview:{scroll_key}:{index}");
                if let Some(clicked) = previews.snapshot(&key, path, text).show(ui, &key) {
                    link = Some(clicked);
                }
            });
        }
    });
    link
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

pub(super) fn paint_diff_side(ui: &mut egui::Ui, paint: DiffSidePaint<'_>) {
    let DiffSidePaint {
        size,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    ui.allocate_ui(size, |ui| {
        ui.set_min_size(size);
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
        if let Some(line) = line {
            paint_diff_line(
                ui,
                DiffPaint {
                    width: size.x,
                    line,
                    gutter,
                    colors,
                    metrics,
                },
            );
        }
    });
}

pub(super) fn diff_row_style(kind: diff::LineKind, colors: DiffColors) -> (Color32, &'static str) {
    match kind {
        diff::LineKind::Insert => (tint(colors.added, 40), "+"),
        diff::LineKind::Delete => (tint(colors.deleted, 40), "-"),
        diff::LineKind::Hunk => (tint(colors.accent, 24), " "),
        diff::LineKind::Equal => (Color32::TRANSPARENT, " "),
    }
}

pub(super) fn diff_code_job(
    line: &diff::DiffLine,
    font: &egui::FontId,
    colors: DiffColors,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping::no_max_width(),
        break_on_newline: false,
        ..Default::default()
    };
    for span in &line.spans {
        let intra = match (line.kind, span.intra) {
            (diff::LineKind::Insert, diff::Intra::Change) => tint(colors.added, 90),
            (diff::LineKind::Delete, diff::Intra::Change) => tint(colors.deleted, 90),
            _ => Color32::TRANSPARENT,
        };
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: if line.kind == diff::LineKind::Hunk {
                    colors.accent
                } else {
                    Color32::from_rgb(span.rgb[0], span.rgb[1], span.rgb[2])
                },
                background: intra,
                ..Default::default()
            },
        );
    }
    if job.text.is_empty() {
        job.append(
            " ",
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: colors.text,
                ..Default::default()
            },
        );
    }
    job
}

pub(super) fn fill_diff_row(ui: &egui::Ui, rect: egui::Rect, gutter_w: f32, bg: Color32) {
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let gutter =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
    ui.painter()
        .rect_filled(gutter, 0.0, Color32::from_black_alpha(40));
}

pub(super) fn paint_diff_number(ui: &egui::Ui, paint: DiffNumberPaint<'_>) {
    let DiffNumberPaint {
        top,
        right,
        number,
        metrics,
        color,
    } = paint;
    let Some(number) = number else {
        return;
    };
    let galley = ui
        .painter()
        .layout_no_wrap(number.to_string(), metrics.font.clone(), color);
    ui.painter().galley(
        egui::pos2(right - galley.size().x, top).round(),
        galley,
        color,
    );
}

pub(super) fn paint_diff_gutter(ui: &egui::Ui, paint: DiffGutterPaint<'_>) {
    let DiffGutterPaint {
        rect,
        cols,
        line,
        sign,
        colors,
        metrics,
    } = paint;
    if line.kind == diff::LineKind::Hunk {
        return;
    }
    let weak = ui.visuals().weak_text_color();
    if let Some(right) = cols.old_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.old_no,
                metrics,
                color: weak,
            },
        );
    }
    if let Some(right) = cols.new_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.new_no,
                metrics,
                color: weak,
            },
        );
    }
    if sign != " " {
        let color = match line.kind {
            diff::LineKind::Insert => colors.added,
            diff::LineKind::Delete => colors.deleted,
            diff::LineKind::Hunk | diff::LineKind::Equal => weak,
        };
        let galley = ui
            .painter()
            .layout_no_wrap(sign.to_owned(), metrics.font.clone(), color);
        ui.painter().galley(
            egui::pos2(rect.left() + cols.sign, rect.top()).round(),
            galley,
            color,
        );
    }
}

pub(super) fn paint_diff_line(ui: &mut egui::Ui, paint: DiffPaint<'_>) {
    let DiffPaint {
        width,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    let (bg, sign) = diff_row_style(line.kind, colors);
    let cols = gutter_cols(metrics, gutter);
    let code = ui
        .painter()
        .layout_job(diff_code_job(line, &metrics.font, colors));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, metrics.row_h), egui::Sense::hover());
    fill_diff_row(ui, rect, cols.code, bg);
    paint_diff_gutter(
        ui,
        DiffGutterPaint {
            rect,
            cols,
            line,
            sign,
            colors,
            metrics,
        },
    );
    ui.painter().galley(
        egui::pos2(rect.left() + cols.code, rect.top()).round(),
        code,
        colors.text,
    );
}

pub(super) fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
