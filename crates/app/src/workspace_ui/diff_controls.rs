//! Native hunk controls layered over the virtualized diff rows.
use super::super::*;
use super::diff_paint::{diff_row_count, diff_row_height};
use diff::actions::Kind;

#[derive(Default)]
pub(super) struct Interaction {
    pub action: Option<(usize, Kind)>,
    pub open_line: Option<u32>,
    pub gap: Option<(usize, diff::GapPart)>,
}

pub(super) struct JumpInput<'a> {
    pub(super) ui: &'a egui::Ui,
    pub(super) doc: &'a diff::DiffDocument,
    pub(super) split: bool,
    pub(super) aligned: bool,
    pub(super) review: &'a mut diff::review::State,
    pub(super) scroll: &'a [egui::scroll_area::ScrollAreaOutput<()>],
    pub(super) sync: &'a mut f32,
}

pub(super) fn jump(input: JumpInput<'_>) {
        let JumpInput { ui, doc, split, aligned, review, scroll, sync } = input;

    if !review.jump {
        return;
    }
    review.jump = false;
    let Some(hunk) = review.selected.and_then(|index| doc.hunks.get(index)) else {
        return;
    };
    let top = hunk_row(hunk, split, aligned) as f32 * diff_row_height(ui);
    for output in scroll {
        let mut state = output.state;
        let mut offset = state.unclamped_offset();
        offset.y = (top - diff_row_height(ui))
            .max(0.0)
            .min((output.content_size.y - output.inner_rect.height()).max(0.0));
        if split {
            *sync = offset.y;
        }
        super::write_scroll_offset(&mut state, offset);
        state.store(ui.ctx(), output.id);
    }
    ui.ctx().request_repaint();
}

/// First row of a hunk. In the aligned editable split both panes are laid
/// out on the buffer's lines, so a hunk sits at its right-side line.
fn hunk_row(hunk: &diff::review::Hunk, split: bool, aligned: bool) -> usize {
    if aligned {
        hunk.new.start
    } else {
        hunk.row(split)
    }
}

/// The changed line a diff row shows on one side: `(old side, line)` for
/// removed lines (left / unified deletions) and added lines (right /
/// unified insertions); `None` for context, gaps, and fillers.
 struct ChangedLineInput<'a> {
     doc: &'a diff::DiffDocument,
     split: bool,
     side: usize,
     row: usize,
}

fn changed_line(input: ChangedLineInput<'_>) -> Option<(bool, usize)> {
        let ChangedLineInput { doc, split, side, row } = input;

    let line = if split {
        let row = doc.split.get(row)?;
        if side == 0 {
            row.left.as_ref()
        } else {
            row.right.as_ref()
        }
    } else {
        doc.unified.get(row)
    }?;
    let index = |number: Option<u32>| usize::try_from(number?).ok()?.checked_sub(1);
    match line.kind {
        diff::LineKind::Delete => Some((true, index(line.old_no)?)),
        diff::LineKind::Insert => Some((false, index(line.new_no)?)),
        _ => None,
    }
}

pub(super) fn change_color(
    hunk: &diff::review::Hunk,
    colors: super::diff_paint::DiffColors,
) -> Color32 {
    if hunk.replacement() {
        colors.accent
    } else if hunk.old.is_empty() {
        colors.added
    } else {
        colors.deleted
    }
}

/// Fill a smooth from/to band as a triangle strip, including zero-line sides.
pub(super) struct PaintConnectorInput<'a> {
    pub(super) ui: &'a egui::Ui,
    pub(super) left: egui::Rect,
    pub(super) right: egui::Rect,
    pub(super) clip: egui::Rect,
    pub(super) color: Color32,
}

pub(super) fn paint_connector(input: PaintConnectorInput<'_>) {
        let PaintConnectorInput { ui, left, right, clip, color } = input;

    let mut mesh = egui::Mesh::default();
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for step in 0_u32..=12 {
        let t = step as f32 / 12.0;
        let blend = t * t * (3.0 - 2.0 * t);
        let x = egui::lerp(left.right()..=right.left(), t);
        let a = egui::pos2(x, egui::lerp(left.top()..=right.top(), blend));
        let b = egui::pos2(x, egui::lerp(left.bottom()..=right.bottom(), blend));
        top.push(a);
        bottom.push(b);
        mesh.colored_vertex(a, super::diff_paint::tint(color, 36));
        mesh.colored_vertex(b, super::diff_paint::tint(color, 36));
        if step > 0 {
            let index = step.saturating_mul(2);
            mesh.add_triangle(index.saturating_sub(2), index.saturating_sub(1), index);
            mesh.add_triangle(index.saturating_sub(1), index.saturating_add(1), index);
        }
    }
    let painter = ui.painter().with_clip_rect(clip);
    painter.add(egui::Shape::mesh(mesh));
    let stroke = egui::Stroke::new(1.0, super::diff_paint::tint(color, 110));
    painter.add(egui::Shape::line(top, stroke));
    painter.add(egui::Shape::line(bottom, stroke));
}

pub(super) struct PaintInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) doc: &'a diff::DiffDocument,
    pub(super) split: bool,
    pub(super) staged: bool,
    pub(super) editor_pane: bool,
    pub(super) enabled: bool,
    pub(super) review: &'a mut diff::review::State,
    pub(super) scroll: &'a [egui::scroll_area::ScrollAreaOutput<()>],
    pub(super) rail: egui::Rect,
    pub(super) unified_gutter: egui::Rect,
    pub(super) colors: super::diff_paint::DiffColors,
}

pub(super) fn paint(input: PaintInput<'_>) -> Interaction {
        let PaintInput { ui, doc, split, staged, editor_pane, enabled, review, scroll, rail, unified_gutter, colors } = input;

    let mut result = Interaction::default();
    let Some(last) = scroll.last() else {
        return result;
    };
    let Some(first) = scroll.first() else {
        return result;
    };
    let row_h = diff_row_height(ui);
    // Editable split: both panes are the buffer's lines (see `align_left`).
    let aligned = split && editor_pane;
    let count = if aligned {
        doc.right_text.split_inclusive('\n').count().max(1)
    } else {
        diff_row_count(doc, split).max(1)
    };
    let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
    // In the live editor views the rows are the full file, so document gaps
    // do not exist there; their click widgets would also cover editor rows
    // and steal clicks (and keyboard focus) from the text editors.
    for (index, gap) in doc.gaps.iter().enumerate().filter(|_| !editor_pane) {
        for (side, output) in scroll.iter().enumerate() {
            let y = output.inner_rect.top() + gap.row(split) as f32 * row_h
                - output.state.unclamped_offset().y;
            let rect = egui::Rect::from_min_size(
                egui::pos2(output.inner_rect.left(), y),
                egui::vec2(output.inner_rect.width(), row_h),
            )
            .intersect(output.inner_rect);
            if !rect.is_positive() {
                continue;
            }
            let click = ui
                .interact(
                    rect,
                    ui.scope_id().with(("diff-gap", index, side)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!("Show {} unchanged lines", gap.old.len()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("diff-gap-{index}"), rect);
            if click.clicked() {
                result.gap = Some((index, diff::GapPart::All));
            }
            // Long gaps also expand a step at a time from either edge.
            if gap.old.len() > diff::GAP_STEP.saturating_mul(2) {
                let step = diff::GAP_STEP;
                for (offset, part, label, tip) in [
                    (
                        2.0,
                        diff::GapPart::Top,
                        "↓",
                        format!("Show {step} more lines below the change above"),
                    ),
                    (
                        1.0,
                        diff::GapPart::Bottom,
                        "↑",
                        format!("Show {step} more lines above the change below"),
                    ),
                ] {
                    let button = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - offset * row_h * 1.6, rect.top()),
                        egui::vec2(row_h * 1.5, row_h),
                    )
                    .intersect(rect);
                    if !button.is_positive() {
                        continue;
                    }
                    let response = ui
                        .interact(
                            button,
                            ui.scope_id().with(("diff-gap-step", index, side, label)),
                            egui::Sense::click(),
                        )
                        .on_hover_text(tip)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.hovered() {
                        ui.painter().rect_filled(
                            button,
                            2.0,
                            ui.visuals().widgets.hovered.weak_bg_fill,
                        );
                    }
                    ui.painter().text(
                        button.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{label}{step}"),
                        egui::FontId::proportional(11.0),
                        ui.visuals().weak_text_color(),
                    );
                    if response.clicked() {
                        result.gap = Some((index, part));
                    }
                }
            }
        }
    }
    for (index, hunk) in doc.hunks.iter().enumerate() {
        let color = change_color(hunk, colors);
        let fraction = hunk_row(hunk, split, aligned) as f32 / count as f32;
        let marker = egui::Rect::from_center_size(
            egui::pos2(
                rail.center().x,
                rail.top() + fraction * (rail.height() - 6.0) + 3.0,
            ),
            egui::vec2(rail.width(), 6.0),
        );
        let selected = review.selected == Some(index);
        ui.painter().rect_filled(marker, 1.0, color);
        let click = ui
            .interact(
                marker.expand(2.0),
                ui.scope_id().with(("diff-marker", index)),
                egui::Sense::click(),
            )
            .on_hover_text(format!(
                "Change {} of {}",
                index.saturating_add(1),
                doc.hunks.len()
            ));
        if click.clicked() {
            review.select(index);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("diff-marker-{index}"), marker);
        let row_top = hunk_row(hunk, split, aligned) as f32 * row_h;
        let old_height = if aligned {
            // Removed lines have no row of their own once aligned.
            hunk.new.len() as f32 * row_h
        } else if split {
            hunk.old.len() as f32 * row_h
        } else {
            hunk.rows(false).len() as f32 * row_h
        };
        let new_height = if split {
            hunk.new.len() as f32 * row_h
        } else {
            old_height
        };
        let left = egui::Rect::from_min_size(
            egui::pos2(
                first.inner_rect.left(),
                first.inner_rect.top() + row_top - first.state.unclamped_offset().y,
            ),
            egui::vec2(first.inner_rect.width(), old_height.max(1.0)),
        );
        let right = egui::Rect::from_min_size(
            egui::pos2(
                last.inner_rect.left(),
                last.inner_rect.top() + row_top - last.state.unclamped_offset().y,
            ),
            egui::vec2(last.inner_rect.width(), new_height.max(1.0)),
        );
        if !left.intersects(first.inner_rect) && !right.intersects(last.inner_rect) {
            continue;
        }
        if split && !editor_pane {
            let gap_clip = egui::Rect::from_min_max(
                egui::pos2(first.inner_rect.right(), first.inner_rect.top()),
                egui::pos2(last.inner_rect.left(), last.inner_rect.bottom()),
            );
            paint_connector(PaintConnectorInput { ui, left, right, clip: gap_clip, color });
        }
        for (side, (rect, output)) in [(left, first), (right, last)].iter().enumerate() {
            if !split && side == 0 {
                continue;
            }
            let clip = output.inner_rect;
            let visible = rect.intersect(clip);
            if !visible.is_positive() {
                continue;
            }
            // The editor pane's rows are buffer lines, not diff rows: an
            // outline there would frame the wrong lines (unless the panes
            // are aligned on those lines). Rail + tints speak.
            if selected && (aligned || !(editor_pane && side == 1)) {
                ui.painter().with_clip_rect(clip).rect_stroke(
                    *rect,
                    0.0,
                    egui::Stroke::new(1.0, color),
                    egui::StrokeKind::Inside,
                );
            }
            // Observe clicks without covering the text's selection widget.
            if let Some(pos) = pointer.filter(|pos| visible.contains(*pos))
                && ui.input(|input| input.pointer.primary_clicked())
            {
                review.selected = Some(index);
                review.confirm_discard = None;
                // Alt+click picks single changed lines for line-level
                // stage/unstage/discard (painted views only).
                if ui.input(|input| input.modifiers.alt) && !editor_pane {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "Row index from UI coordinates."
                    )]
                    let row = ((pos.y - output.inner_rect.top()
                        + output.state.unclamped_offset().y)
                        / row_h)
                        .floor()
                        .max(0.0) as usize;
                    if let Some((old, line)) = changed_line(ChangedLineInput { doc, split, side, row })
                        && (if old { &hunk.old } else { &hunk.new }).contains(&line)
                    {
                        review.toggle_line(index, old, line);
                    }
                }
            }
            // Mark picked lines in this hunk.
            if let Some((picked_hunk, lines)) = &review.picked
                && *picked_hunk == index
            {
                for row in hunk.rows(split) {
                    let Some((old, line)) = changed_line(ChangedLineInput { doc, split, side, row }) else {
                        continue;
                    };
                    if (if old { &lines.old } else { &lines.new }).contains(&line) {
                        let y = output.inner_rect.top() + row as f32 * row_h
                            - output.state.unclamped_offset().y;
                        ui.painter().with_clip_rect(clip).text(
                            egui::pos2(output.inner_rect.left() + 2.0, y + row_h / 2.0),
                            egui::Align2::LEFT_CENTER,
                            "✓",
                            egui::FontId::proportional(row_h * 0.8),
                            color,
                        );
                    }
                }
            }
        }
        let band = left.union(right);
        if !selected && !pointer.is_some_and(|pos| band.contains(pos)) {
            continue;
        }
        let width = if split {
            (right.left() - left.right()).max(48.0)
        } else {
            unified_gutter.width()
        };
        let x = if split {
            left.right()
        } else {
            unified_gutter.left()
        };
        let y = left.top().max(first.inner_rect.top()).min(
            (first.inner_rect.bottom() - appearance::TOOLBAR_BUTTON).max(first.inner_rect.top()),
        );
        let controls = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(width, appearance::TOOLBAR_BUTTON),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("diff-hunk-actions", index))
                .max_rect(controls)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(controls.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.add_space(((width - 47.0) / 2.0).max(0.0));
                ui.add_enabled_ui(enabled, |ui| {
                    let kind = if staged { Kind::Unstage } else { Kind::Stage };
                    let picked = review
                        .picked
                        .as_ref()
                        .filter(|(picked_hunk, _)| *picked_hunk == index)
                        .map_or(0, |(_, lines)| lines.len());
                    let target = if picked > 0 {
                        format!(
                            "{picked} selected line{}",
                            if picked == 1 { "" } else { "s" }
                        )
                    } else {
                        "this hunk (Alt+click lines to pick some)".to_owned()
                    };
                    let action = appearance::sidebar_action(
                        ui,
                        if staged { "ArrowLeft" } else { "Plus" },
                        &format!("{} {target}", kind.label()),
                    );
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("diff-hunk-action-{index}"),
                        action.rect,
                    );
                    if action.clicked() {
                        result.action = Some((index, kind));
                    }
                    if !staged {
                        let discard = appearance::sidebar_action(
                            ui,
                            "Eraser",
                            &if picked > 0 {
                                format!("Discard {target}…")
                            } else {
                                "Discard this hunk…".to_owned()
                            },
                        );
                        #[cfg(feature = "test-support")]
                        diagnostics::record(
                            ui.ctx(),
                            &format!("diff-hunk-discard-{index}"),
                            discard.rect,
                        );
                        if discard.clicked() {
                            result.action = Some((index, Kind::Discard));
                        }
                    }
                });
            },
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_bands_distinguish_replacements_additions_and_deletions() {
        let colors = super::super::diff_paint::DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            text: Color32::WHITE,
        };
        for (old, new, expected) in [
            ("old\n", "new\n", Color32::BLUE),
            ("", "new\n", Color32::GREEN),
            ("old\n", "", Color32::RED),
        ] {
            let doc =
                diff::build_with_context(std::path::Path::new("file.txt"), old, new, false, 3);
            let hunk = doc.hunks.first().unwrap();
            assert_eq!(change_color(hunk, colors), expected);
            let row = super::super::diff_paint::row_colors(super::super::diff_paint::RowColorsInput { doc: &doc, row: hunk.row(true), split: true, colors });
            if hunk.replacement() {
                assert_eq!(row.added, Color32::BLUE);
                assert_eq!(row.deleted, Color32::BLUE);
            }
        }
    }

    #[test]
    fn next_change_scrolls_to_the_hunk_in_both_views() {
        use std::fmt::Write;
        let mut original = String::new();
        for line in 0..1200 {
            writeln!(original, "line {line}").unwrap();
        }
        let changed = original
            .replace("line 20\n", "first change\n")
            .replace("line 1100\n", "last change\n");
        let doc = diff::build_with_context(
            std::path::Path::new("file.txt"),
            &original,
            &changed,
            false,
            2000,
        );
        for split in [false, true] {
            let ctx = egui::Context::default();
            let mut review = diff::review::State::default();
            review.select(1);
            let mut sync = 0.0;
            let mut ratio = 0.5;
            let mut visible = false;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 500.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let colors = super::super::diff_paint::DiffColors {
                            added: Color32::GREEN,
                            deleted: Color32::RED,
                            accent: Color32::BLUE,
                            text: Color32::WHITE,
                        };
                        let scroll = super::super::diff_paint::paint_diff_document(super::super::diff_paint::PaintDiffDocumentInput { ui, doc: &doc, split, colors, scroll_key: "jump-regression", ratio: &mut ratio, sync: &mut sync });
                        for output in &scroll {
                            let top = output.inner_rect.top()
                                + doc.hunks.get(1).unwrap().row(split) as f32 * diff_row_height(ui)
                                - output.state.unclamped_offset().y;
                            visible = output.state.unclamped_offset().y > 1000.0
                                && output.inner_rect.y_range().contains(top);
                            assert!(
                                !visible || top + diff_row_height(ui) <= output.inner_rect.bottom()
                            );
                        }
                        jump(JumpInput { ui, doc: &doc, split, aligned: false, review: &mut review, scroll: &scroll, sync: &mut sync });
                    },
                );
                output.textures_delta.clear();
            }
            assert!(
                visible,
                "the selected hunk must enter the viewport; split={split}"
            );
        }
    }
}
