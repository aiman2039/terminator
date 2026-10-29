use super::super::super::*;
use super::{ControlsFrame, Interaction, PaneOutput};

/// One collapsed gap as shown in one pane.
struct GapView<'a> {
    index: usize,
    side: usize,
    gap: &'a diff::review::Gap,
    rect: egui::Rect,
}

impl ControlsFrame<'_> {
    /// Click targets that reveal collapsed unchanged lines.
    pub(super) fn paint_gaps(&self, ui: &mut egui::Ui, result: &mut Interaction) {
        for (index, gap) in self.doc.gaps.iter().enumerate() {
            for (side, output) in self.scroll.iter().enumerate() {
                let Some(rect) = self.gap_rect(gap, output) else {
                    continue;
                };
                let view = GapView {
                    index,
                    side,
                    gap,
                    rect,
                };
                self.paint_gap(ui, &view, result);
            }
        }
    }

    fn gap_rect(&self, gap: &diff::review::Gap, output: &PaneOutput) -> Option<egui::Rect> {
        let row_h = self.row_h;
        let y = output.inner_rect.top() + gap.row(self.split) as f32 * row_h
            - output.state.unclamped_offset().y;
        let rect = egui::Rect::from_min_size(
            egui::pos2(output.inner_rect.left(), y),
            egui::vec2(output.inner_rect.width(), row_h),
        )
        .intersect(output.inner_rect);
        rect.is_positive().then_some(rect)
    }

    fn paint_gap(&self, ui: &mut egui::Ui, view: &GapView<'_>, result: &mut Interaction) {
        let GapView {
            index,
            side,
            gap,
            rect,
        } = *view;
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
            self.paint_gap_steps(ui, view, result);
        }
    }

    fn paint_gap_steps(&self, ui: &mut egui::Ui, view: &GapView<'_>, result: &mut Interaction) {
        let GapView {
            index, side, rect, ..
        } = *view;
        let row_h = self.row_h;
        let step = diff::GAP_STEP;
        // Same monospace font as the rows, so the step labels line up.
        let font = super::super::diff_paint::diff_row_font(ui);
        let label_w = ui
            .painter()
            .layout_no_wrap(format!("↓{step}"), font.clone(), Color32::WHITE)
            .size()
            .x;
        let button_w = label_w + row_h * 0.5;
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
            let left = rect.right() - offset * (button_w + 2.0);
            let button = egui::Rect::from_min_size(
                super::super::diff_paint::snap(ui, egui::pos2(left, rect.top())),
                egui::vec2(button_w, row_h),
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
                ui.painter()
                    .rect_filled(button, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            ui.painter().text(
                button.center(),
                egui::Align2::CENTER_CENTER,
                format!("{label}{step}"),
                font.clone(),
                ui.visuals().weak_text_color(),
            );
            if response.clicked() {
                result.gap = Some((index, part));
            }
        }
    }
}
