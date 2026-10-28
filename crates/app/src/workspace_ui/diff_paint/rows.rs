use super::*;

pub(in super::super) fn paint_diff_side(ui: &mut egui::Ui, paint: DiffSidePaint<'_>) {
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

pub(crate) struct ComparisonHeadersInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) ratio: f32,
    pub(crate) path: &'a std::path::Path,
    pub(crate) editable: bool,
    pub(crate) staged: bool,
}

pub(in super::super) fn comparison_headers(input: ComparisonHeadersInput<'_>) {
    let ComparisonHeadersInput {
        ui,
        doc,
        split,
        ratio,
        path,
        editable,
        staged,
    } = input;

    let full = ui.available_rect_before_wrap();
    let header =
        egui::Rect::from_min_size(full.min, egui::vec2((full.width() - 10.0).max(1.0), 54.0));
    let usable = (header.width() - DIFF_SPLIT_GAP).max(1.0);
    let min = DIFF_SPLIT_MIN_PANE.min(usable / 2.0);
    let left_width = (ratio * usable).clamp(min, usable - min);
    let labels = if split {
        vec![
            (
                egui::Rect::from_min_size(header.min, egui::vec2(left_width, header.height())),
                doc.left_label.clone(),
            ),
            (
                egui::Rect::from_min_max(
                    egui::pos2(header.left() + left_width + DIFF_SPLIT_GAP, header.top()),
                    header.max,
                ),
                doc.right_label.clone(),
            ),
        ]
    } else {
        vec![(header, format!("{} → {}", doc.left_label, doc.right_label))]
    };
    for (index, (rect, label)) in labels.into_iter().enumerate() {
        let side_editable = editable && (!split || index == 1);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.horizontal(|ui| {
                ui.strong(label);
                ui.weak(if side_editable {
                    "Editable"
                } else if staged {
                    "Staged · Read-only"
                } else {
                    "Read-only"
                });
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(doc.source.as_ref().map_or_else(
                        || crate::services::compact_path(path),
                        |source| source.path.display().to_string(),
                    ))
                    .weak(),
                )
                .truncate(),
            )
            .on_hover_text(path.display().to_string());
        });
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
    }
    ui.allocate_rect(header, egui::Sense::hover());
}

pub(in super::super) fn diff_code_job(
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
                    colors.text.gamma_multiply(0.55)
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

pub(crate) struct FillDiffRowInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) rect: egui::Rect,
    pub(crate) gutter_w: f32,
    pub(crate) bg: Color32,
}

pub(in super::super) fn fill_diff_row(input: FillDiffRowInput<'_>) {
    let FillDiffRowInput {
        ui,
        rect,
        gutter_w,
        bg,
    } = input;

    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let gutter =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
    ui.painter()
        .rect_filled(gutter, 0.0, Color32::from_black_alpha(40));
}

pub(in super::super) fn paint_diff_number(ui: &egui::Ui, paint: DiffNumberPaint<'_>) {
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

pub(in super::super) fn paint_diff_gutter(ui: &egui::Ui, paint: DiffGutterPaint<'_>) {
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

pub(in super::super) fn paint_diff_line(ui: &mut egui::Ui, paint: DiffPaint<'_>) {
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
    fill_diff_row(FillDiffRowInput {
        ui,
        rect,
        gutter_w: cols.code,
        bg,
    });
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
    let pos = egui::pos2(rect.left() + cols.code, rect.top()).round();
    let response = ui.interact(
        egui::Rect::from_min_size(pos, code.size()),
        ui.auto_id_with("diff-code-selection"),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, code.text()));
    egui::text_selection::LabelSelectionState::label_text_selection(
        ui,
        &response,
        pos,
        code,
        colors.text,
        egui::Stroke::NONE,
    );
}
