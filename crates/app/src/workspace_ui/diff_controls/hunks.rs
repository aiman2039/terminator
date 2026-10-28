use super::super::super::*;
use super::{
    ChangedLineInput, ControlsFrame, ControlsState, PaintConnectorInput, PaneOutput, change_color,
    changed_line, hunk_row, paint_connector,
};
use diff::actions::Kind;

/// One hunk's on-screen bands this frame.
struct HunkBand<'a> {
    index: usize,
    hunk: &'a diff::review::Hunk,
    color: Color32,
    selected: bool,
    left: egui::Rect,
    right: egui::Rect,
}

/// A hunk band as shown in one pane.
struct SideView<'a> {
    band: &'a HunkBand<'a>,
    side: usize,
    rect: egui::Rect,
    output: &'a PaneOutput,
}

impl ControlsFrame<'_> {
    /// Rail marker, outline, line picks, and action buttons for one hunk.
    pub(super) fn paint_hunk(
        &self,
        ui: &mut egui::Ui,
        (index, hunk): (usize, &diff::review::Hunk),
        state: &mut ControlsState<'_>,
    ) {
        let color = change_color(hunk, self.colors);
        let selected = state.review.selected == Some(index);
        self.paint_rail_marker(ui, (index, hunk, color), state.review);
        let (left, right) = self.hunk_bands(hunk);
        if !left.intersects(self.first.inner_rect) && !right.intersects(self.last.inner_rect) {
            return;
        }
        let band = HunkBand {
            index,
            hunk,
            color,
            selected,
            left,
            right,
        };
        if self.split && !self.editor_pane {
            let gap_clip = egui::Rect::from_min_max(
                egui::pos2(self.first.inner_rect.right(), self.first.inner_rect.top()),
                egui::pos2(self.last.inner_rect.left(), self.last.inner_rect.bottom()),
            );
            paint_connector(PaintConnectorInput {
                ui,
                left,
                right,
                clip: gap_clip,
                color,
            });
        }
        for (side, (rect, output)) in [(left, self.first), (right, self.last)]
            .into_iter()
            .enumerate()
        {
            let view = SideView {
                band: &band,
                side,
                rect,
                output,
            };
            self.paint_hunk_side(ui, &view, state.review);
        }
        let union = left.union(right);
        if !selected && !self.pointer.is_some_and(|pos| union.contains(pos)) {
            return;
        }
        self.hunk_actions(ui, &band, state);
    }

    fn paint_rail_marker(
        &self,
        ui: &egui::Ui,
        (index, hunk, color): (usize, &diff::review::Hunk, Color32),
        review: &mut diff::review::State,
    ) {
        let rail = self.rail;
        let fraction = hunk_row(hunk, self.split, self.aligned) as f32 / self.count as f32;
        let marker = egui::Rect::from_center_size(
            egui::pos2(
                rail.center().x,
                rail.top() + fraction * (rail.height() - 6.0) + 3.0,
            ),
            egui::vec2(rail.width(), 6.0),
        );
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
                self.doc.hunks.len()
            ));
        if click.clicked() {
            review.select(index);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("diff-marker-{index}"), marker);
    }

    /// The hunk's old-side band in the first pane and new-side band in
    /// the last pane, in screen coordinates.
    fn hunk_bands(&self, hunk: &diff::review::Hunk) -> (egui::Rect, egui::Rect) {
        let (row_h, first, last) = (self.row_h, self.first, self.last);
        let row_top = hunk_row(hunk, self.split, self.aligned) as f32 * row_h;
        let old_height = if self.aligned {
            // Removed lines have no row of their own once aligned.
            hunk.new.len() as f32 * row_h
        } else if self.split {
            hunk.old.len() as f32 * row_h
        } else {
            hunk.rows(false).len() as f32 * row_h
        };
        let new_height = if self.split {
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
        (left, right)
    }

    fn paint_hunk_side(
        &self,
        ui: &egui::Ui,
        view: &SideView<'_>,
        review: &mut diff::review::State,
    ) {
        let SideView {
            band,
            side,
            rect,
            output,
        } = *view;
        if !self.split && side == 0 {
            return;
        }
        let clip = output.inner_rect;
        let visible = rect.intersect(clip);
        if !visible.is_positive() {
            return;
        }
        // The editor pane's rows are buffer lines, not diff rows: an
        // outline there would frame the wrong lines (unless the panes are
        // aligned on those lines). Rail + tints speak.
        if band.selected && (self.aligned || !(self.editor_pane && side == 1)) {
            ui.painter().with_clip_rect(clip).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, band.color),
                egui::StrokeKind::Inside,
            );
        }
        // Observe clicks without covering the text's selection widget.
        if let Some(pos) = self.pointer.filter(|pos| visible.contains(*pos))
            && ui.input(|input| input.pointer.primary_clicked())
        {
            review.selected = Some(band.index);
            review.confirm_discard = None;
            // Alt+click picks single changed lines for line-level
            // stage/unstage/discard (painted views only).
            if ui.input(|input| input.modifiers.alt) && !self.editor_pane {
                self.pick_line_at(view, pos, review);
            }
        }
        self.mark_picked_lines(ui, view, review);
    }

    fn pick_line_at(&self, view: &SideView<'_>, pos: egui::Pos2, review: &mut diff::review::State) {
        let SideView {
            band, side, output, ..
        } = *view;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Row index from UI coordinates."
        )]
        let row = ((pos.y - output.inner_rect.top() + output.state.unclamped_offset().y)
            / self.row_h)
            .floor()
            .max(0.0) as usize;
        let hunk = band.hunk;
        if let Some((old, line)) = changed_line(ChangedLineInput {
            doc: self.doc,
            split: self.split,
            side,
            row,
        }) && (if old { &hunk.old } else { &hunk.new }).contains(&line)
        {
            review.toggle_line(band.index, old, line);
        }
    }

    /// Mark picked lines in this hunk.
    fn mark_picked_lines(&self, ui: &egui::Ui, view: &SideView<'_>, review: &diff::review::State) {
        let SideView {
            band, side, output, ..
        } = *view;
        let Some((picked_hunk, lines)) = &review.picked else {
            return;
        };
        if *picked_hunk != band.index {
            return;
        }
        let row_h = self.row_h;
        let clip = output.inner_rect;
        for row in band.hunk.rows(self.split) {
            let Some((old, line)) = changed_line(ChangedLineInput {
                doc: self.doc,
                split: self.split,
                side,
                row,
            }) else {
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
                    band.color,
                );
            }
        }
    }

    /// Stage/unstage and discard buttons over the hovered or selected hunk.
    fn hunk_actions(&self, ui: &mut egui::Ui, band: &HunkBand<'_>, state: &mut ControlsState<'_>) {
        let (left, right, first) = (band.left, band.right, self.first);
        let width = if self.split {
            (right.left() - left.right()).max(48.0)
        } else {
            self.unified_gutter.width()
        };
        let x = if self.split {
            left.right()
        } else {
            self.unified_gutter.left()
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
                .id_salt(("diff-hunk-actions", band.index))
                .max_rect(controls)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(controls.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.add_space(((width - 47.0) / 2.0).max(0.0));
                ui.add_enabled_ui(self.enabled, |ui| {
                    self.hunk_action_buttons(ui, band.index, state);
                });
            },
        );
    }

    fn hunk_action_buttons(&self, ui: &mut egui::Ui, index: usize, state: &mut ControlsState<'_>) {
        let staged = self.staged;
        let kind = if staged { Kind::Unstage } else { Kind::Stage };
        let picked = state
            .review
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
        diagnostics::record(ui.ctx(), &format!("diff-hunk-action-{index}"), action.rect);
        if action.clicked() {
            state.result.action = Some((index, kind));
        }
        if staged {
            return;
        }
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
            state.result.action = Some((index, Kind::Discard));
        }
    }
}
