//! The diff view is the editor: the split right pane is a live,
//! syntax-highlighted buffer and the unified view edits lines at their row.
//! There is no separate edit mode to enter or leave.
use super::super::*;
use std::sync::Arc;

mod right;
mod row_ops;
mod unified;

pub(crate) use right::PaintRightEditorInput;
pub(super) use right::paint_right_editor;
#[cfg(test)]
pub(super) use row_ops::{RowOp, apply_row_op};
pub(crate) use unified::PaintUnifiedEditorInput;
pub(super) use unified::paint_unified_editor;

fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80)
}

/// Layout job whose text is exactly the buffer text (a `TextEdit` requirement),
/// colored from cached syntect spans with per-line diff tints.
pub(crate) struct HighlightedJobInput<'a> {
    pub(crate) text: &'a str,
    pub(crate) spans: &'a [Vec<diff::DiffSpan>],
    pub(crate) valid: &'a [bool],
    pub(crate) tints: &'a [Option<Color32>],
    pub(crate) font: egui::FontId,
    pub(crate) fallback: Color32,
    pub(crate) wrap_width: f32,
}

pub(super) fn highlighted_job(input: HighlightedJobInput<'_>) -> egui::text::LayoutJob {
    let HighlightedJobInput {
        text,
        spans,
        valid,
        tints,
        font,
        fallback,
        wrap_width,
    } = input;

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
    // Rehash the whole buffer only after an edit (or a palette change),
    // not on every frame.
    let palette = buffer_key("", colors);
    if buffer.stale || buffer.right_spans.is_none() || buffer.palette != palette {
        buffer.key = buffer_key(&buffer.text, colors);
        buffer.palette = palette;
        buffer.stale = false;
    }
    let key_hash = buffer.key;
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

/// The editable split view's left pane, aligned row-for-row with the
/// buffer; rebuilt only when the buffer (or palette) changes.
pub(super) fn aligned_left(
    buffer: &mut diff::review::Edit,
    path: &std::path::Path,
    colors: super::diff_paint::DiffColors,
) -> Arc<diff::AlignedLeft> {
    let _ = ensure_cache(buffer, path, colors);
    let key = buffer.key;
    if let Some((cached, aligned)) = &buffer.aligned_left
        && *cached == key
    {
        return Arc::clone(aligned);
    }
    let left_spans = Arc::clone(buffer.left_spans.get_or_insert_with(|| {
        Arc::new(if buffer.document.left_text.len() > MAX_HIGHLIGHT_BYTES {
            Vec::new()
        } else {
            diff::highlight_spans(path, &buffer.document.left_text)
        })
    }));
    let aligned = Arc::new(diff::align_left(
        &buffer.document.left_text,
        &buffer.text,
        &left_spans,
    ));
    buffer.aligned_left = Some((key, Arc::clone(&aligned)));
    aligned
}

/// Red rules where the buffer dropped original lines (they have no row
/// on the right); hovering one lists the deleted text.
pub(crate) struct PaintDeletedMarkersInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) output: &'a egui::scroll_area::ScrollAreaOutput<()>,
    pub(crate) deleted: &'a [(usize, Vec<String>)],
    pub(crate) row_h: f32,
    pub(crate) color: Color32,
    pub(crate) key: &'a str,
}

pub(super) fn paint_deleted_markers(input: PaintDeletedMarkersInput<'_>) {
    let PaintDeletedMarkersInput {
        ui,
        output,
        deleted,
        row_h,
        color,
        key,
    } = input;

    let clip = output.inner_rect;
    let offset = output.state.unclamped_offset().y;
    for (index, (row, lines)) in deleted.iter().enumerate() {
        let y = clip.top() + *row as f32 * row_h - offset;
        if y < clip.top() - 4.0 || y > clip.bottom() + 4.0 {
            continue;
        }
        ui.painter()
            .with_clip_rect(clip)
            .hline(clip.x_range(), y, egui::Stroke::new(2.0, color));
        let hover = egui::Rect::from_min_max(
            egui::pos2(clip.left(), y - 3.0),
            egui::pos2(clip.right(), y + 3.0),
        )
        .intersect(clip);
        if hover.is_positive() {
            let mut text = format!(
                "{} deleted line{}",
                lines.len(),
                if lines.len() == 1 { "" } else { "s" }
            );
            for line in lines.iter().take(20) {
                text.push('\n');
                text.push_str(line);
            }
            ui.interact(
                hover,
                egui::Id::unique((key, "deleted-marker", index)),
                egui::Sense::hover(),
            )
            .on_hover_text(egui::RichText::new(text).monospace());
        }
    }
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
