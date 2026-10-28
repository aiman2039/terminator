//! Native hunk controls layered over the virtualized diff rows.
use super::super::*;
use super::diff_paint::{diff_row_count, diff_row_height};
use diff::actions::Kind;

mod gaps;
mod hunks;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Interaction {
    pub action: Option<(usize, Kind)>,
    pub open_line: Option<u32>,
    pub gap: Option<(usize, diff::GapPart)>,
}

pub(crate) struct JumpInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) aligned: bool,
    pub(crate) review: &'a mut diff::review::State,
    pub(crate) scroll: &'a [egui::scroll_area::ScrollAreaOutput<()>],
    pub(crate) sync: &'a mut f32,
}

pub(super) fn jump(input: JumpInput<'_>) {
    let JumpInput {
        ui,
        doc,
        split,
        aligned,
        review,
        scroll,
        sync,
    } = input;

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
pub(crate) struct ChangedLineInput<'a> {
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) side: usize,
    pub(crate) row: usize,
}

fn changed_line(input: ChangedLineInput<'_>) -> Option<(bool, usize)> {
    let ChangedLineInput {
        doc,
        split,
        side,
        row,
    } = input;

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
pub(crate) struct PaintConnectorInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) left: egui::Rect,
    pub(crate) right: egui::Rect,
    pub(crate) clip: egui::Rect,
    pub(crate) color: Color32,
}

pub(super) fn paint_connector(input: PaintConnectorInput<'_>) {
    let PaintConnectorInput {
        ui,
        left,
        right,
        clip,
        color,
    } = input;

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

pub(crate) struct PaintInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) doc: &'a diff::DiffDocument,
    pub(crate) split: bool,
    pub(crate) staged: bool,
    pub(crate) editor_pane: bool,
    pub(crate) enabled: bool,
    pub(crate) review: &'a mut diff::review::State,
    pub(crate) scroll: &'a [egui::scroll_area::ScrollAreaOutput<()>],
    pub(crate) rail: egui::Rect,
    pub(crate) unified_gutter: egui::Rect,
    pub(crate) colors: super::diff_paint::DiffColors,
}

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// Per-frame layout shared by the gap and hunk control steps.
struct ControlsFrame<'a> {
    doc: &'a diff::DiffDocument,
    split: bool,
    staged: bool,
    editor_pane: bool,
    enabled: bool,
    scroll: &'a [PaneOutput],
    first: &'a PaneOutput,
    last: &'a PaneOutput,
    rail: egui::Rect,
    unified_gutter: egui::Rect,
    colors: super::diff_paint::DiffColors,
    aligned: bool,
    count: usize,
    row_h: f32,
    pointer: Option<egui::Pos2>,
}

/// The review the controls edit, and what they asked for.
struct ControlsState<'a> {
    review: &'a mut diff::review::State,
    result: Interaction,
}

pub(super) fn paint(input: PaintInput<'_>) -> Interaction {
    let PaintInput {
        ui,
        doc,
        split,
        staged,
        editor_pane,
        enabled,
        review,
        scroll,
        rail,
        unified_gutter,
        colors,
    } = input;

    let mut state = ControlsState {
        review,
        result: Interaction::default(),
    };
    let (Some(first), Some(last)) = (scroll.first(), scroll.last()) else {
        return state.result;
    };
    let row_h = diff_row_height(ui);
    // Editable split: both panes are the buffer's lines (see `align_left`).
    let aligned = split && editor_pane;
    let count = if aligned {
        doc.right_text.split_inclusive('\n').count().max(1)
    } else {
        diff_row_count(doc, split).max(1)
    };
    let frame = ControlsFrame {
        doc,
        split,
        staged,
        editor_pane,
        enabled,
        scroll,
        first,
        last,
        rail,
        unified_gutter,
        colors,
        aligned,
        count,
        row_h,
        pointer: ui.ctx().input(|input| input.pointer.hover_pos()),
    };
    // In the live editor views the rows are the full file, so document gaps
    // do not exist there; their click widgets would also cover editor rows
    // and steal clicks (and keyboard focus) from the text editors.
    if !editor_pane {
        frame.paint_gaps(ui, &mut state.result);
    }
    for (index, hunk) in doc.hunks.iter().enumerate() {
        frame.paint_hunk(ui, (index, hunk), &mut state);
    }
    state.result
}
