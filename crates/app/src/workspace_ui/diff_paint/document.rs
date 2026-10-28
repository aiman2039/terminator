use super::*;

pub(in super::super) fn diff_content_width(input: DiffContentWidthInput<'_>) -> f32 {
    let DiffContentWidthInput {
        ui,
        doc,
        split,
        metrics,
        scroll_key,
    } = input;

    let cache_id = egui::Id::unique(("diff-width", scroll_key, split));
    let fingerprint = egui::Id::unique((
        doc.generation,
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

pub(crate) struct PaintDiffDocumentInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) colors: DiffColors,
    pub(crate) scroll_key: &'a str,
    pub(crate) ratio: &'a mut f32,
    pub(crate) sync: &'a mut f32,
}

pub(in super::super) fn paint_diff_document(
    input: PaintDiffDocumentInput<'_>,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let PaintDiffDocumentInput {
        ui,
        doc,
        split,
        colors,
        scroll_key,
        ratio,
        sync,
    } = input;

    let rows = diff_row_count(doc, split);
    let digits = diff_doc_digits(doc, split);
    let metrics = DiffMetrics::measure(ui, digits);
    let content = diff_content_width(DiffContentWidthInput {
        ui,
        doc,
        split,
        metrics: &metrics,
        scroll_key,
    });
    if split {
        paint_diff_split(PaintDiffSplitInput {
            ui,
            doc,
            colors,
            metrics: &metrics,
            content,
            rows,
            scroll_key,
            ratio,
            sync,
        })
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
                                    colors: row_colors(RowColorsInput {
                                        doc,
                                        row: index,
                                        split: false,
                                        colors,
                                    }),
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

pub(in super::super) const DIFF_SPLIT_GAP: f32 = 72.0;
pub(in super::super) const DIFF_SPLIT_MIN_PANE: f32 = 40.0;

/// `(left, gap, right)` rects for a split diff at the given pane ratio.
pub(in super::super) fn split_rects(
    full: egui::Rect,
    ratio: f32,
) -> (egui::Rect, egui::Rect, egui::Rect) {
    let usable = (full.width() - DIFF_SPLIT_GAP).max(1.0);
    let min_pane = DIFF_SPLIT_MIN_PANE.min(usable / 2.0);
    let left_width = (ratio * usable).clamp(min_pane, usable - min_pane);
    let left = egui::Rect::from_min_size(full.min, egui::vec2(left_width, full.height()));
    let gap = egui::Rect::from_min_size(
        egui::pos2(full.left() + left_width, full.top()),
        egui::vec2(DIFF_SPLIT_GAP, full.height()),
    );
    let right = egui::Rect::from_min_max(egui::pos2(gap.right(), full.top()), full.max);
    (left, gap, right)
}

#[derive(Clone, Copy)]
pub(in super::super) enum SplitPane {
    Left,
    Right,
}

// Each side owns its horizontal scroll so the divider can resize them without
// clipping long lines; the vertical offset is mirrored so both sides stay in step.
pub(crate) struct PaintDiffSplitInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) colors: DiffColors,
    pub(crate) metrics: &'a DiffMetrics,
    pub(crate) content: f32,
    pub(crate) rows: usize,
    pub(crate) scroll_key: &'a str,
    pub(crate) ratio: &'a mut f32,
    pub(crate) sync: &'a mut f32,
}

pub(in super::super) fn paint_diff_split(
    input: PaintDiffSplitInput<'_>,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let PaintDiffSplitInput {
        ui,
        doc,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        ratio,
        sync,
    } = input;

    let full = ui.available_rect_before_wrap();
    let usable = (full.width() - DIFF_SPLIT_GAP).max(1.0);
    let min_pane = DIFF_SPLIT_MIN_PANE.min(usable / 2.0);
    let (left_rect, gap_rect, right_rect) = split_rects(full, *ratio);
    // Paint both panes at the same frame-start offset so corresponding lines
    // stay aligned; whichever pane the user scrolled becomes next frame's shared
    // offset.
    let frame_sync = *sync;
    let left = paint_diff_pane(PaintDiffPaneInput {
        ui,
        rect: left_rect,
        doc,
        pane: SplitPane::Left,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        sync: frame_sync,
    });
    let right = paint_diff_pane(PaintDiffPaneInput {
        ui,
        rect: right_rect,
        doc,
        pane: SplitPane::Right,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        sync: frame_sync,
    });
    // Adopt the pane the pointer is over so sub-pixel input accumulates there
    // and an inactive pane's clamp cannot override the active pane. With the
    // pointer elsewhere (keyboard, divider drag), fall back to whichever pane
    // moved further from the frame-start offset.
    let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
    let over_left = pointer.is_some_and(|pos| left_rect.contains(pos));
    let over_right = pointer.is_some_and(|pos| right_rect.contains(pos));
    let left_offset = left.state.unclamped_offset().y;
    let right_offset = right.state.unclamped_offset().y;
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
    paint_diff_divider(PaintDiffDividerInput {
        ui,
        full,
        gap_rect,
        scroll_key,
        ratio,
        colors,
        usable,
        min_pane,
    });
    vec![left, right]
}

pub(crate) struct PaintDiffDividerInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) full: egui::Rect,
    pub(crate) gap_rect: egui::Rect,
    pub(crate) scroll_key: &'a str,
    pub(crate) ratio: &'a mut f32,
    pub(crate) colors: DiffColors,
    pub(crate) usable: f32,
    pub(crate) min_pane: f32,
}

pub(in super::super) fn paint_diff_divider(input: PaintDiffDividerInput<'_>) {
    let PaintDiffDividerInput {
        ui,
        full,
        gap_rect,
        scroll_key,
        ratio,
        colors,
        usable,
        min_pane,
    } = input;
    ui.allocate_rect(full, egui::Sense::hover());
    let handle =
        egui::Rect::from_center_size(gap_rect.center(), egui::vec2(6.0, gap_rect.height()));
    let response = ui.interact(
        handle,
        ui.scope_id().with(("diff-split-drag", scroll_key)),
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
    diagnostics::record(ui.ctx(), "diff-split-handle", handle);
}

pub(crate) struct PaintDiffPaneInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) rect: egui::Rect,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) pane: SplitPane,
    pub(crate) colors: DiffColors,
    pub(crate) metrics: &'a DiffMetrics,
    pub(crate) content: f32,
    pub(crate) rows: usize,
    pub(crate) scroll_key: &'a str,
    pub(crate) sync: f32,
}

pub(in super::super) fn paint_diff_pane(
    input: PaintDiffPaneInput<'_>,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let PaintDiffPaneInput {
        ui,
        rect,
        doc,
        pane,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        sync,
    } = input;

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
                                size: egui::vec2(content.max(rect.width()), metrics.row_h),
                                line: doc.split.get(index).and_then(|row| match pane {
                                    SplitPane::Left => row.left.as_ref(),
                                    SplitPane::Right => row.right.as_ref(),
                                }),
                                gutter: match pane {
                                    SplitPane::Left => DiffGutter::Old,
                                    SplitPane::Right => DiffGutter::New,
                                },
                                colors: row_colors(RowColorsInput {
                                    doc,
                                    row: index,
                                    split: true,
                                    colors,
                                }),
                                metrics,
                            },
                        );
                    }
                })
        },
    )
    .inner
}
pub(crate) struct PaintMarkdownDiffPreviewInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) previews: &'a mut markdown::Previews,
    pub(crate) path: &'a std::path::Path,
    pub(crate) scroll_key: &'a str,
}

pub(in super::super) fn paint_markdown_diff_preview(
    input: PaintMarkdownDiffPreviewInput<'_>,
) -> Option<markdown::Link> {
    let PaintMarkdownDiffPreviewInput {
        ui,
        doc,
        previews,
        path,
        scroll_key,
    } = input;

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
