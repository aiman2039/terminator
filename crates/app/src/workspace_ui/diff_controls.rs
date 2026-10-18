//! Native hunk controls layered over the virtualized diff rows.
use super::super::*;
use super::diff_paint::{diff_row_count, diff_row_height};
use diff::actions::Kind;

#[derive(Default)]
pub(super) struct Interaction {
    pub action: Option<(usize, Kind)>,
    pub open_line: Option<u32>,
    pub gap: Option<usize>,
}

pub(super) fn jump(
    ui: &egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    review: &mut diff::review::State,
    scroll: &[egui::scroll_area::ScrollAreaOutput<()>],
    sync: &mut f32,
) {
    if !review.jump {
        return;
    }
    review.jump = false;
    let Some(hunk) = review.selected.and_then(|index| doc.hunks.get(index)) else {
        return;
    };
    #[allow(clippy::cast_precision_loss)]
    let top = hunk.row(split) as f32 * diff_row_height(ui);
    for output in scroll {
        let mut state = output.state;
        state.offset.y = (top - diff_row_height(ui))
            .max(0.0)
            .min((output.content_size.y - output.inner_rect.height()).max(0.0));
        if split {
            *sync = state.offset.y;
        }
        state.store(ui.ctx(), output.id);
    }
    ui.ctx().request_repaint();
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
pub(super) fn paint_connector(
    ui: &egui::Ui,
    left: egui::Rect,
    right: egui::Rect,
    clip: egui::Rect,
    color: Color32,
) {
    let mut mesh = egui::Mesh::default();
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for step in 0_u32..=12 {
        #[allow(clippy::cast_precision_loss)]
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

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    staged: bool,
    enabled: bool,
    review: &mut diff::review::State,
    scroll: &[egui::scroll_area::ScrollAreaOutput<()>],
    rail: egui::Rect,
    unified_gutter: egui::Rect,
    colors: super::diff_paint::DiffColors,
) -> Interaction {
    let mut result = Interaction::default();
    let Some(last) = scroll.last() else {
        return result;
    };
    let Some(first) = scroll.first() else {
        return result;
    };
    let row_h = diff_row_height(ui);
    let count = diff_row_count(doc, split).max(1);
    let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
    let body = ui.interact(
        last.inner_rect,
        ui.id().with("diff-open-line"),
        egui::Sense::click(),
    );
    if body.double_clicked()
        && let Some(pos) = body.interact_pointer_pos()
    {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let row = ((pos.y - last.inner_rect.top() + last.state.offset.y) / row_h).max(0.0) as usize;
        result.open_line = if split {
            doc.split
                .get(row)
                .and_then(|row| row.right.as_ref())
                .and_then(|line| line.new_no)
        } else {
            doc.unified.get(row).and_then(|line| line.new_no)
        };
    }
    for (index, gap) in doc.gaps.iter().enumerate() {
        for (side, output) in scroll.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let y = output.inner_rect.top() + gap.row(split) as f32 * row_h - output.state.offset.y;
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
                    ui.id().with(("diff-gap", index, side)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!("Show {} unchanged lines", gap.old.len()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("diff-gap-{index}"), rect);
            if click.clicked() {
                result.gap = Some(index);
            }
        }
    }
    for (index, hunk) in doc.hunks.iter().enumerate() {
        let color = change_color(hunk, colors);
        #[allow(clippy::cast_precision_loss)]
        let fraction = hunk.row(split) as f32 / count as f32;
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
                ui.id().with(("diff-marker", index)),
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
        #[allow(clippy::cast_precision_loss)]
        let row_top = hunk.row(split) as f32 * row_h;
        #[allow(clippy::cast_precision_loss)]
        let old_height = if split {
            hunk.old.len() as f32 * row_h
        } else {
            hunk.rows(false).len() as f32 * row_h
        };
        #[allow(clippy::cast_precision_loss)]
        let new_height = if split {
            hunk.new.len() as f32 * row_h
        } else {
            old_height
        };
        let left = egui::Rect::from_min_size(
            egui::pos2(
                first.inner_rect.left(),
                first.inner_rect.top() + row_top - first.state.offset.y,
            ),
            egui::vec2(first.inner_rect.width(), old_height.max(1.0)),
        );
        let right = egui::Rect::from_min_size(
            egui::pos2(
                last.inner_rect.left(),
                last.inner_rect.top() + row_top - last.state.offset.y,
            ),
            egui::vec2(last.inner_rect.width(), new_height.max(1.0)),
        );
        if !left.intersects(first.inner_rect) && !right.intersects(last.inner_rect) {
            continue;
        }
        if split {
            let gap_clip = egui::Rect::from_min_max(
                egui::pos2(first.inner_rect.right(), first.inner_rect.top()),
                egui::pos2(last.inner_rect.left(), last.inner_rect.bottom()),
            );
            paint_connector(ui, left, right, gap_clip, color);
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
            if selected {
                ui.painter().with_clip_rect(clip).rect_stroke(
                    *rect,
                    0.0,
                    egui::Stroke::new(1.0, color),
                    egui::StrokeKind::Inside,
                );
            }
            let response = ui.interact(
                visible,
                ui.id().with(("diff-hunk", index, side)),
                egui::Sense::click(),
            );
            if response.clicked() {
                review.selected = Some(index);
                review.confirm_discard = None;
            }
            if response.double_clicked()
                && (split && side == 1 || !split)
                && let Some(pos) = response.interact_pointer_pos()
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let row = ((pos.y - output.inner_rect.top() + output.state.offset.y) / row_h)
                    .max(0.0) as usize;
                result.open_line = if split {
                    doc.split
                        .get(row)
                        .and_then(|row| row.right.as_ref())
                        .and_then(|line| line.new_no)
                } else {
                    doc.unified.get(row).and_then(|line| line.new_no)
                };
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
                    let action = appearance::sidebar_action(
                        ui,
                        if staged { "ArrowLeft" } else { "Plus" },
                        &format!("{} this hunk", kind.label()),
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
                        let discard =
                            appearance::sidebar_action(ui, "Eraser", "Discard this hunk…");
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
    #![allow(clippy::unwrap_used)]
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
            let row = super::super::diff_paint::row_colors(&doc, hunk.row(true), true, colors);
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
                        let scroll = super::super::diff_paint::paint_diff_document(
                            ui,
                            &doc,
                            split,
                            colors,
                            "jump-regression",
                            &mut ratio,
                            &mut sync,
                        );
                        for output in &scroll {
                            #[allow(clippy::cast_precision_loss)]
                            let top = output.inner_rect.top()
                                + doc.hunks.get(1).unwrap().row(split) as f32 * diff_row_height(ui)
                                - output.state.offset.y;
                            visible = output.state.offset.y > 1000.0
                                && output.inner_rect.y_range().contains(top);
                            assert!(
                                !visible || top + diff_row_height(ui) <= output.inner_rect.bottom()
                            );
                        }
                        jump(ui, &doc, split, &mut review, &scroll, &mut sync);
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
