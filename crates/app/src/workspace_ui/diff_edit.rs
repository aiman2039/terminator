//! The diff view is the editor: the split right pane is a live,
//! syntax-highlighted buffer and the unified view edits lines at their row.
//! There is no separate edit mode to enter or leave.
use super::super::*;
use std::sync::Arc;

fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80)
}

/// Layout job whose text is exactly the buffer text (a `TextEdit` requirement),
/// colored from cached syntect spans with per-line diff tints.
pub(super) fn highlighted_job(
    text: &str,
    spans: &[Vec<diff::DiffSpan>],
    valid: &[bool],
    tints: &[Option<Color32>],
    font: egui::FontId,
    fallback: Color32,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    for (index, part) in text.split_inclusive('\n').enumerate() {
        let background = tints
            .get(index)
            .copied()
            .flatten()
            .unwrap_or(Color32::TRANSPARENT);
        let content = part.trim_end_matches(['\n', '\r']);
        let ending = part.get(content.len()..).unwrap_or("");
        let line_spans: &[diff::DiffSpan] = spans.get(index).map_or(&[], Vec::as_slice);
        if valid.get(index).copied().unwrap_or(false) && !line_spans.is_empty() {
            for span in line_spans {
                job.append(
                    &span.text,
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color: Color32::from_rgb(span.rgb[0], span.rgb[1], span.rgb[2]),
                        background,
                        ..Default::default()
                    },
                );
            }
        } else if !content.is_empty() {
            job.append(
                content,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: fallback,
                    background,
                    ..Default::default()
                },
            );
        }
        if !ending.is_empty() {
            job.append(
                ending,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: fallback,
                    background,
                    ..Default::default()
                },
            );
        }
    }
    if text.is_empty() {
        job.append(
            "",
            0.0,
            egui::TextFormat {
                font_id: font,
                color: fallback,
                ..Default::default()
            },
        );
    }
    job
}

/// Replace one 0-based line of `text`, preserving its original line ending.
/// Out-of-range lines leave the text untouched.
pub(super) fn replace_line(text: &str, line: usize, replacement: &str) -> String {
    if text.is_empty() && line == 0 {
        return replacement.to_owned();
    }
    text.split_inclusive('\n')
        .enumerate()
        .map(|(index, part)| {
            if index == line {
                let ending = if part.ends_with("\r\n") {
                    "\r\n"
                } else if part.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                format!("{replacement}{ending}")
            } else {
                part.to_owned()
            }
        })
        .collect()
}

/// Tints for the *current* buffer text, diffed live against the left side:
/// accent for replaced lines, green for inserted lines. Recomputed only when
/// the buffer text changes, never per frame.
pub(super) fn live_tints(
    left: &str,
    right: &str,
    colors: super::diff_paint::DiffColors,
) -> Vec<Option<Color32>> {
    let mut tints = vec![None; right.split_inclusive('\n').count().max(1)];
    let diff = similar::TextDiff::from_lines(left, right);
    for op in diff.ops() {
        let color = match op.tag() {
            similar::DiffTag::Replace | similar::DiffTag::Insert => colors.added,
            _ => continue,
        };
        for index in op.new_range() {
            if let Some(slot) = tints.get_mut(index) {
                *slot = Some(tint(color));
            }
        }
    }
    tints
}

/// Unified rows for the whole file, diffed live from the left text against
/// the current buffer: equal lines, inserted/replaced lines, and deleted
/// lines in place. Typing anywhere re-diffs on the next frame, so blocks
/// appear, move, and disappear in real time.
pub(super) fn live_rows(left: &str, right: &str) -> Vec<diff::review::LiveRow> {
    use diff::review::{LiveRow, LiveTint};
    let diff = similar::TextDiff::from_lines(left, right);
    let left_lines: Vec<&str> = left.split_inclusive('\n').collect();
    let mut rows = Vec::new();
    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {
                for offset in 0..op.old_range().len() {
                    rows.push(LiveRow::Edit {
                        old: Some(op.old_range().start.saturating_add(offset)),
                        new: op.new_range().start.saturating_add(offset),
                        changed: None,
                    });
                }
            }
            similar::DiffTag::Delete => {
                for old in op.old_range() {
                    rows.push(LiveRow::Gone {
                        old,
                        text: left_lines
                            .get(old)
                            .map(|part| part.trim_end_matches(['\n', '\r']).to_owned())
                            .unwrap_or_default(),
                    });
                }
            }
            similar::DiffTag::Insert => {
                for new in op.new_range() {
                    rows.push(LiveRow::Edit {
                        old: None,
                        new,
                        changed: Some(LiveTint::Added),
                    });
                }
            }
            similar::DiffTag::Replace => {
                for old in op.old_range() {
                    rows.push(LiveRow::Gone {
                        old,
                        text: left_lines
                            .get(old)
                            .map(|part| part.trim_end_matches(['\n', '\r']).to_owned())
                            .unwrap_or_default(),
                    });
                }
                for new in op.new_range() {
                    rows.push(LiveRow::Edit {
                        old: None,
                        new,
                        changed: Some(LiveTint::Changed),
                    });
                }
            }
        }
    }
    if rows.is_empty() {
        rows.push(LiveRow::Edit {
            old: None,
            new: 0,
            changed: None,
        });
    }
    rows
}

/// Cache key for a buffer revision: text plus the tint palette.
fn buffer_key(text: &str, colors: super::diff_paint::DiffColors) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    for color in [colors.added, colors.deleted, colors.accent] {
        color.to_array().hash(&mut hasher);
    }
    hasher.finish()
}

/// Above this size the editor paints plain rather than re-highlighting the
/// whole file on each keystroke (mirrors the native editor's cap).
const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

/// The split diff's right pane is a live editor: no edit mode, the buffer is
/// edited in place with syntax highlighting and diff tints. Returns a
/// `ScrollAreaOutput<()>` shaped like the painted panes so hunk controls and
/// connectors keep working unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_right_editor(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    buffer: &mut diff::review::Edit,
    path: &std::path::Path,
    colors: super::diff_paint::DiffColors,
    sync: f32,
    key: &str,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let (spans, valid, tints, _rows) = ensure_cache(buffer, path, colors);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let fallback = colors.text;
    // Reserve a number gutter inside the pane, painted after the editor.
    let line_count = buffer.text.split_inclusive('\n').count().max(1);
    #[allow(clippy::cast_precision_loss)]
    let digits = line_count.to_string().len().max(1) as f32;
    let (digit_w, row_h) = ui
        .ctx()
        .fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
    #[allow(clippy::cast_precision_loss)]
    let gutter_w = digit_w * digits + 10.0;
    let editor_rect =
        egui::Rect::from_min_max(egui::pos2(rect.left() + gutter_w, rect.top()), rect.max);
    let output = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(editor_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::both()
                    .id_salt((key, "diff-right-editor"))
                    .auto_shrink([false, false])
                    .vertical_scroll_offset(sync)
                    .show(ui, |ui| {
                        let mut layouter =
                            |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                                ui.ctx().fonts_mut(|fonts| {
                                    fonts.layout_job(highlighted_job(
                                        text.as_str(),
                                        &spans,
                                        &valid,
                                        &tints,
                                        font.clone(),
                                        fallback,
                                        wrap_width,
                                    ))
                                })
                            };
                        let editor = egui::TextEdit::multiline(&mut buffer.text)
                            .id(egui::Id::unique((key, "diff-right-text")))
                            .font(egui::TextStyle::Monospace)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .layouter(&mut layouter)
                            .desired_width(f32::INFINITY)
                            .show(ui);
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "diff-right-editor", editor.response.rect);
                        #[cfg(not(feature = "test-support"))]
                        let _ = &editor;
                    })
            },
        )
        .inner;
    // Full-width background blocks behind changed lines, on the background
    // layer so they sit under the text: the line *and* its block highlight,
    // recomputed live from the buffer on every revision.
    if row_h > 0.0 {
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(rect);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::arithmetic_side_effects
        )]
        let first = (output.state.unclamped_offset().y / row_h).floor().max(0.0) as usize;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::arithmetic_side_effects
        )]
        let visible = (output.inner_rect.height() / row_h).ceil() as usize + 1;
        for (index, tint) in tints.iter().enumerate().skip(first).take(visible) {
            let Some(tint) = tint else { continue };
            #[allow(clippy::cast_precision_loss)]
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
    // Paint buffer line numbers in the reserved gutter.
    if row_h > 0.0 {
        let painter = ui.painter().with_clip_rect(rect);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::arithmetic_side_effects
        )]
        let first = (output.state.unclamped_offset().y / row_h).floor().max(0.0) as usize;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::arithmetic_side_effects
        )]
        let visible = (output.inner_rect.height() / row_h).ceil() as usize + 1;
        for index in first..line_count.min(first.saturating_add(visible)) {
            #[allow(clippy::cast_precision_loss)]
            let center_y = output.inner_rect.top() + index as f32 * row_h
                - output.state.unclamped_offset().y
                + row_h / 2.0;
            painter.text(
                egui::pos2(editor_rect.left() - 5.0, center_y),
                egui::Align2::RIGHT_CENTER,
                index.saturating_add(1).to_string(),
                font.clone(),
                ui.visuals().weak_text_color(),
            );
        }
    }
    egui::scroll_area::ScrollAreaOutput {
        inner: (),
        id: output.id,
        state: output.state,
        content_size: output.content_size,
        inner_rect: output.inner_rect,
    }
}

type CacheArcs = (
    Arc<Vec<Vec<diff::DiffSpan>>>,
    Arc<Vec<bool>>,
    Arc<Vec<Option<Color32>>>,
    Arc<Vec<diff::review::LiveRow>>,
);

/// Recompute the syntax/tint cache only when the buffer revision changes;
/// returns cheap `Arc` clones for layouters.
fn ensure_cache(
    buffer: &mut diff::review::Edit,
    path: &std::path::Path,
    colors: super::diff_paint::DiffColors,
) -> CacheArcs {
    let key_hash = buffer_key(&buffer.text, colors);
    if buffer
        .right_spans
        .as_ref()
        .is_none_or(|(cached, _, _)| *cached != key_hash)
    {
        let spans = if buffer.text.len() > MAX_HIGHLIGHT_BYTES {
            Vec::new()
        } else {
            diff::highlight_spans(path, &buffer.text)
        };
        // Validate each line's spans once per revision; layouters trust
        // these flags instead of rebuilding every line's text per frame.
        let valid: Vec<bool> = buffer
            .text
            .split_inclusive('\n')
            .enumerate()
            .map(|(index, part)| {
                let content = part.trim_end_matches(['\n', '\r']);
                spans.get(index).is_some_and(|line_spans| {
                    !line_spans.is_empty()
                        && line_spans.iter().map(|span| span.text.len()).sum::<usize>()
                            == content.len()
                        && line_spans
                            .iter()
                            .flat_map(|span| span.text.chars())
                            .eq(content.chars())
                })
            })
            .collect();
        buffer.right_spans = Some((key_hash, Arc::new(spans), Arc::new(valid)));
        buffer.live_tints = Some((
            key_hash,
            Arc::new(live_tints(&buffer.document.left_text, &buffer.text, colors)),
        ));
        buffer.live_rows = Some((
            key_hash,
            Arc::new(live_rows(&buffer.document.left_text, &buffer.text)),
        ));
    }
    let (right_spans, valid) = buffer.right_spans.as_ref().map_or_else(
        || (Arc::new(Vec::new()), Arc::new(Vec::new())),
        |(_, spans, valid)| (Arc::clone(spans), Arc::clone(valid)),
    );
    let right_tints = buffer
        .live_tints
        .as_ref()
        .map_or_else(|| Arc::new(Vec::new()), |(_, tints)| Arc::clone(tints));
    let rows = buffer
        .live_rows
        .as_ref()
        .map_or_else(|| Arc::new(Vec::new()), |(_, rows)| Arc::clone(rows));
    (right_spans, valid, right_tints, rows)
}

/// Structural edit requested from a unified row editor.
pub(super) enum RowOp {
    /// Enter: split the line at the cursor into head/tail.
    Split {
        line: usize,
        head: String,
        tail: String,
    },
    /// Backspace at column 0: join this line onto the previous one.
    JoinPrev { line: usize },
    /// Delete at end of line: pull the next line up.
    JoinNext { line: usize },
}

/// Apply a structural edit to whole-buffer text, preserving its line ending
/// style. Out-of-range lines leave the text untouched.
pub(super) fn apply_row_op(text: &str, op: &RowOp) -> String {
    let sep = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split(sep).map(str::to_owned).collect();
    match op {
        RowOp::Split { line, head, tail } => {
            if let Some(slot) = lines.get_mut(*line) {
                slot.clone_from(head);
                lines.insert(line.saturating_add(1), tail.clone());
            }
        }
        RowOp::JoinPrev { line } => {
            if *line > 0 && *line < lines.len() {
                let current = lines.remove(*line);
                if let Some(prev) = line.checked_sub(1).and_then(|prev| lines.get_mut(prev)) {
                    prev.push_str(&current);
                }
            }
        }
        RowOp::JoinNext { line } => {
            if line.saturating_add(1) < lines.len() {
                let next = lines.remove(line.saturating_add(1));
                if let Some(current) = lines.get_mut(*line) {
                    current.push_str(&next);
                }
            }
        }
    }
    lines.join(sep)
}

/// Record the pre-edit text as one undo unit. Edits within 0.8s of the last
/// snapshot belong to the same burst and don't add another unit.
pub(super) fn record_undo(buffer: &mut diff::review::Edit, old_text: String, now: f64) {
    if now - buffer.burst > 0.8 || buffer.undo.is_empty() {
        buffer.undo.push(old_text);
        if buffer.undo.len() > 100 {
            buffer.undo.remove(0);
        }
    }
    buffer.burst = now;
    buffer.redo.clear();
}

/// Undo one unit. Returns true when the text changed.
pub(super) fn apply_undo(buffer: &mut diff::review::Edit) -> bool {
    let Some(previous) = buffer.undo.pop() else {
        return false;
    };
    let current = std::mem::replace(&mut buffer.text, previous);
    buffer.redo.push(current);
    buffer.burst = 0.0;
    true
}

/// Redo one unit. Returns true when the text changed.
pub(super) fn apply_redo(buffer: &mut diff::review::Edit) -> bool {
    let Some(next) = buffer.redo.pop() else {
        return false;
    };
    let current = std::mem::replace(&mut buffer.text, next);
    buffer.undo.push(current);
    buffer.burst = 0.0;
    true
}

/// The unified diff is editable everywhere: rows are the live diff of the
/// whole file (see `live_rows`), every buffer row hosts its editor at its
/// own row, deleted rows stay painted in place, and changed blocks carry a
/// full-width background. No hunk windows, no double-click, no mode.
pub(super) fn paint_unified_editor(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    buffer: &mut diff::review::Edit,
    path: &std::path::Path,
    colors: super::diff_paint::DiffColors,
    key: &str,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    use super::diff_paint::{
        DiffGutter, DiffGutterPaint, DiffMetrics, DiffPaint, diff_content_width, diff_row_style,
        fill_diff_row, gutter_cols, paint_diff_gutter, paint_diff_line,
    };
    use diff::review::LiveRow;
    // Buffer-level undo/redo, but only when a unified row editor holds
    // focus (never steal Cmd+Z from e.g. the commit message), and consumed
    // here so the row editor does not also apply its own per-line undo.
    let command_z = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Z,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command || modifiers.ctrl
            )
        })
    });
    if command_z {
        let focused = ui.memory(|memory| memory.focused());
        let line_count = buffer.text.split_inclusive('\n').count();
        let ours = focused.is_some_and(|id| {
            (1..=line_count.saturating_add(2)).any(|n| {
                id == egui::Id::unique((key, "unified-line", u32::try_from(n).unwrap_or(1)))
            })
        });
        if ours {
            let mut shift_command = egui::Modifiers::COMMAND;
            shift_command.shift = true;
            let redo_pressed = ui.input_mut(|input| input.consume_key(shift_command, egui::Key::Z));
            let undo_pressed = !redo_pressed
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
            if redo_pressed {
                apply_redo(buffer);
            } else if undo_pressed {
                apply_undo(buffer);
            }
        }
    }
    if buffer.left_spans.is_none() {
        buffer.left_spans = Some(Arc::new(
            if buffer.document.left_text.len() > MAX_HIGHLIGHT_BYTES {
                Vec::new()
            } else {
                diff::highlight_spans(path, &buffer.document.left_text)
            },
        ));
    }
    let left_spans = buffer.left_spans.clone().unwrap_or_default();
    let (spans, valid, _tints, rows) = ensure_cache(buffer, path, colors);
    let buffer_lines = buffer.text.split_inclusive('\n').count();
    let left_lines = buffer.document.left_text.split_inclusive('\n').count();
    let digits = u32::try_from(left_lines.max(buffer_lines).to_string().len()).unwrap_or(4);
    let metrics = DiffMetrics::measure(ui, digits.max(1));
    let content = diff_content_width(ui, doc, false, &metrics, key);
    let width = content.max(ui.available_width());
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let fallback = colors.text;
    let mut pending: Vec<(usize, String)> = Vec::new();
    let mut row_op: Option<RowOp> = None;
    let mut focus_after: Option<(u32, usize)> = None;
    let output = egui::ScrollArea::both()
        .id_salt((key, "unified-editor"))
        .auto_shrink([false, false])
        .show_rows(ui, metrics.row_h, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(width);
            for index in range {
                let Some(row) = rows.get(index) else {
                    continue;
                };
                match row {
                    LiveRow::Gone { old, text } => {
                        let line = diff::DiffLine {
                            kind: diff::LineKind::Delete,
                            old_no: u32::try_from(*old).ok().map(|n| n.saturating_add(1)),
                            new_no: None,
                            spans: left_spans
                                .get(*old)
                                .cloned()
                                .filter(|spans| {
                                    spans.iter().map(|s| s.text.as_str()).collect::<String>()
                                        == *text
                                })
                                .unwrap_or_else(|| {
                                    vec![diff::DiffSpan {
                                        text: text.clone(),
                                        rgb: [fallback.r(), fallback.g(), fallback.b()],
                                        intra: diff::Intra::None,
                                    }]
                                }),
                        };
                        paint_diff_line(
                            ui,
                            DiffPaint {
                                width,
                                line: &line,
                                gutter: DiffGutter::Unified,
                                colors,
                                metrics: &metrics,
                            },
                        );
                    }
                    LiveRow::Edit { old, new, changed } => {
                        let line_index = *new;
                        let kind = if changed.is_some() {
                            diff::LineKind::Insert
                        } else {
                            diff::LineKind::Equal
                        };
                        let line = diff::DiffLine {
                            kind,
                            old_no: old
                                .and_then(|n| u32::try_from(n).ok())
                                .map(|n| n.saturating_add(1)),
                            new_no: u32::try_from(line_index).ok().map(|n| n.saturating_add(1)),
                            spans: Vec::new(),
                        };
                        let (bg, sign) = diff_row_style(kind, colors);
                        let bg = match changed {
                            Some(_) => super::diff_paint::tint(colors.added, 95),
                            None => bg,
                        };
                        let cols = gutter_cols(&metrics, DiffGutter::Unified);
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(width, metrics.row_h),
                            egui::Sense::hover(),
                        );
                        fill_diff_row(ui, rect, cols.code, bg);
                        paint_diff_gutter(
                            ui,
                            DiffGutterPaint {
                                rect,
                                cols,
                                line: &line,
                                sign,
                                colors,
                                metrics: &metrics,
                            },
                        );
                        let code_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + cols.code, rect.top()),
                            rect.max,
                        );
                        let buffer_line = buffer
                            .text
                            .split_inclusive('\n')
                            .nth(line_index)
                            .map(|part| part.trim_end_matches(['\n', '\r']).to_owned())
                            .unwrap_or_default();
                        let mut local = buffer_line.clone();
                        let line_spans: Vec<diff::DiffSpan> =
                            spans.get(line_index).cloned().unwrap_or_default();
                        let line_valid = valid.get(line_index).copied().unwrap_or(false);
                        let new_no = line.new_no.unwrap_or(1);
                        let editor_id = egui::Id::unique((key, "unified-line", new_no));
                        // Virtualized rows can lose the text widget's click to
                        // the dock. Hold focus across press and release so the
                        // dock cannot clear it before the next keyboard event.
                        let row_pressed = ui.input(|input| {
                            input.events.iter().any(|event| {
                                matches!(
                                    event,
                                    egui::Event::PointerButton {
                                        pos,
                                        button: egui::PointerButton::Primary,
                                        ..
                                    } if code_rect.contains(*pos)
                                )
                            })
                        });
                        if row_pressed {
                            ui.memory_mut(|memory| memory.request_focus(editor_id));
                        }
                        // Enter surrenders a single-line editor's focus during
                        // `show`, so capture focus before it runs.
                        let focused_before =
                            ui.memory(|memory| memory.focused() == Some(editor_id));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(code_rect), |ui| {
                            let mut layouter =
                                |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                                    ui.ctx().fonts_mut(|fonts| {
                                        fonts.layout_job(highlighted_job(
                                            text.as_str(),
                                            std::slice::from_ref(&line_spans),
                                            &[line_valid],
                                            &[bg_to_layout(bg)],
                                            font.clone(),
                                            fallback,
                                            wrap_width,
                                        ))
                                    })
                                };
                            let editor = egui::TextEdit::singleline(&mut local)
                                .id(editor_id)
                                .font(egui::TextStyle::Monospace)
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .layouter(&mut layouter)
                                .desired_width(f32::INFINITY)
                                .show(ui);
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                &format!("diff-unified-edit-{new_no}"),
                                editor.response.rect,
                            );
                            #[cfg(not(feature = "test-support"))]
                            let _ = &editor;
                            if row_pressed {
                                editor.response.request_focus();
                            }
                            // Structural editing, like a real text editor:
                            // Enter splits, Backspace/Delete join lines.
                            // TextEdit already applied any in-line change, so
                            // an unchanged line with the cursor at an edge
                            // means the key was structural, not textual.
                            if focused_before && local == buffer_line {
                                let range = editor.state.cursor.char_range();
                                let (min, max) = range.map_or((0, 0), |r| {
                                    (
                                        r.primary.index.0.min(r.secondary.index.0),
                                        r.primary.index.0.max(r.secondary.index.0),
                                    )
                                });
                                let char_len = local.chars().count();
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    let head: String = local.chars().take(min).collect();
                                    let tail: String = local.chars().skip(max).collect();
                                    row_op = Some(RowOp::Split {
                                        line: line_index,
                                        head,
                                        tail,
                                    });
                                    focus_after = Some((new_no.saturating_add(1), 0));
                                } else if ui.input(|i| i.key_pressed(egui::Key::Backspace))
                                    && min == 0
                                    && max == 0
                                    && line_index > 0
                                {
                                    row_op = Some(RowOp::JoinPrev { line: line_index });
                                    focus_after = Some((
                                        new_no.saturating_sub(1),
                                        buffer
                                            .text
                                            .split_inclusive('\n')
                                            .nth(line_index.saturating_sub(1))
                                            .map_or(0, |part| {
                                                part.trim_end_matches(['\n', '\r']).chars().count()
                                            }),
                                    ));
                                } else if ui.input(|i| i.key_pressed(egui::Key::Delete))
                                    && min == char_len
                                    && max == char_len
                                {
                                    row_op = Some(RowOp::JoinNext { line: line_index });
                                    focus_after = Some((new_no, char_len));
                                }
                            }
                            if editor.response.has_focus() {
                                let target = if ui.input(|i| i.key_pressed(egui::Key::ArrowDown))
                                    && line_index.saturating_add(1) < buffer_lines
                                {
                                    Some(new_no.saturating_add(1))
                                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp))
                                    && new_no > 1
                                {
                                    Some(new_no.saturating_sub(1))
                                } else {
                                    None
                                };
                                if let Some(next) = target {
                                    ui.memory_mut(|memory| {
                                        memory.request_focus(egui::Id::unique((
                                            key,
                                            "unified-line",
                                            next,
                                        )));
                                    });
                                }
                            }
                        });
                        if local != buffer_line {
                            pending.push((line_index, local));
                        }
                    }
                }
            }
        });
    if !pending.is_empty() || row_op.is_some() {
        let old_text = buffer.text.clone();
        for (line_index, text) in pending {
            buffer.text = replace_line(&buffer.text, line_index, &text);
        }
        if let Some(op) = &row_op {
            buffer.text = apply_row_op(&buffer.text, op);
        }
        if buffer.text != old_text {
            let now = ui.input(|input| input.time);
            record_undo(buffer, old_text, now);
        }
    }
    if let Some((line_no, char_pos)) = focus_after {
        let id = egui::Id::unique((key, "unified-line", line_no));
        ui.memory_mut(|memory| memory.request_focus(id));
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state
                .cursor
                .set_char_range(Some(egui::text_selection::CCursorRange::one(
                    egui::text::CCursor::new(char_pos),
                )));
            state.store(ui.ctx(), id);
        }
    }
    output
}

/// Layout background for one editor line: transparent when unchanged.
fn bg_to_layout(bg: Color32) -> Option<Color32> {
    if bg == Color32::TRANSPARENT {
        None
    } else {
        Some(bg)
    }
}

impl App {
    pub(crate) fn block_unsaved_diff(&mut self, key: Option<&str>) -> bool {
        let dirty = self.diff_reviews.iter().any(|(id, state)| {
            key.is_none_or(|key| key == id)
                && (state.unsaved_edits() || state.saving_edits && state.busy)
        });
        if dirty {
            self.error = Some("Save or discard edits in the diff viewer before closing.".into());
        }
        dirty
    }
}
