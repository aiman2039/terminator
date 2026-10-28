use super::super::super::*;
use super::{StripActions, StripFrame};

/// Per-frame drop context for the strip, after its tabs are laid out.
pub(super) struct StripDrop<'a> {
    pub(super) frame: StripFrame<'a>,
    pub(super) plus_rect: egui::Rect,
    pub(super) strip_rects: &'a [(String, egui::Rect)],
    pub(super) released: bool,
    pub(super) workspace: &'a mut Workspace,
    pub(super) actions: &'a mut StripActions,
}

impl StripDrop<'_> {
    /// Insertion slot under the pointer: past the last tab over "+", else
    /// the gap the pointer is nearest to.
    fn insertion_index(&self, pos: egui::Pos2) -> Option<usize> {
        if self.plus_rect.contains(pos) {
            Some(self.strip_rects.len())
        } else {
            App::strip_insertion_at(&self.frame.strip_rect, self.strip_rects, pos)
        }
    }
}

impl App {
    /// Drop routing, tab-drag cancel, and drop previews for the strip.
    pub(super) fn finish_strip_drags(&mut self, ui: &mut egui::Ui, drop: &mut StripDrop<'_>) {
        self.drop_pane_on_strip(ui, drop);
        self.drop_tab_on_strip(ui, drop);
        if self.drag.tab.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drag.tab = None;
        }
        self.update_strip_drag_hover(ui, drop);
        self.paint_strip_drop_preview(ui, drop);
    }

    /// Dropping a dragged terminal pane onto a strip tab moves it into that
    /// tab (its focused split; the tab content was already previewed on
    /// hover); dropping into a gap between tabs, onto "+", or onto empty
    /// strip background opens it in a fresh top-level tab at that slot.
    fn drop_pane_on_strip(&mut self, ui: &egui::Ui, drop: &mut StripDrop<'_>) {
        let Some(pane) = self.drag.pane.clone() else {
            return;
        };
        if !(drop.frame.drag_same_project && drop.released) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if let Some(group_id) = Self::strip_interior_tab(drop.strip_rects, pos) {
            self.pull_dragged_strip_shell(drop.workspace, &pane);
            let moved = if let Some(source) = &self.drag.pane_source {
                drop.workspace.move_view_to_group(source, &pane, &group_id)
            } else {
                drop.workspace.move_pane_to_group(&pane, &group_id)
            };
            self.finish_strip_pane_drop(drop, &pane, moved);
            return;
        }
        let Some(index) = drop.insertion_index(pos) else {
            return;
        };
        self.pull_dragged_strip_shell(drop.workspace, &pane);
        let moved = if let Some(source) = &self.drag.pane_source {
            drop.workspace
                .move_view_to_new_group(source, &pane, index)
                .is_some()
        } else {
            drop.workspace
                .move_pane_to_new_group_at(&pane, index)
                .is_some()
        };
        self.finish_strip_pane_drop(drop, &pane, moved);
    }

    fn pull_dragged_strip_shell(&mut self, workspace: &mut Workspace, pane: &Tab) {
        if self.drag.pane_from_strip
            && let Tab::Terminal(sid) = pane
        {
            self.pull_strip_shell_into(workspace, sid);
        }
    }

    fn finish_strip_pane_drop(&mut self, drop: &mut StripDrop<'_>, pane: &Tab, moved: bool) {
        if moved {
            self.focus_moved_view(pane, drop.frame.project, drop.workspace);
            self.pending_layout_save = true;
            if let Tab::Terminal(sid) = pane {
                self.active_session = Some(sid.clone());
                self.focus_tab = Some(pane.clone());
            }
            self.pane_index = None;
            drop.actions.switch = None;
        }
        self.drag.preview_origin = None;
        self.end_pane_drag();
    }

    /// Dropping a dragged strip tab reorders it to the insertion slot. A
    /// released tab drag the strip did not consume was dropped outside of
    /// it: cancel without moving anything.
    fn drop_tab_on_strip(&mut self, ui: &egui::Ui, drop: &mut StripDrop<'_>) {
        if !(self.drag.tab.is_some() && drop.released) {
            return;
        }
        if let (Some(dragged), Some(pos)) = (
            self.drag.tab.clone(),
            ui.input(|i| i.pointer.interact_pos()),
        ) && drop.workspace.tabs.iter().any(|tab| tab.id == dragged)
            && let Some(index) = drop.insertion_index(pos)
            && drop.workspace.reorder_group(&dragged, index)
        {
            drop.workspace.active = dragged;
            drop.actions.switch = None;
        }
        self.drag.tab = None;
    }

    /// The dock paints the previewed tab's focused leaf at real size while
    /// a pane hovers a strip tab interior; hovering a gap, "+", or empty
    /// strip background shows the tab ghost instead of the pane snapshot
    /// ghost. Both mirror the drop routing.
    fn update_strip_drag_hover(&mut self, ui: &egui::Ui, drop: &StripDrop<'_>) {
        let strip_pos = (!drop.released && ui.input(|i| i.pointer.any_down()))
            .then(|| ui.input(|i| i.pointer.interact_pos()))
            .flatten();
        let drag_same_project = drop.frame.drag_same_project;
        self.drag.strip_tab_hover = drag_same_project
            && strip_pos
                .is_some_and(|pos| Self::strip_interior_tab(drop.strip_rects, pos).is_some());
        self.drag.strip_new_tab_hover = drag_same_project
            && strip_pos.is_some_and(|pos| {
                Self::strip_interior_tab(drop.strip_rects, pos).is_none()
                    && (drop.plus_rect.contains(pos)
                        || Self::strip_insertion_at(&drop.frame.strip_rect, drop.strip_rects, pos)
                            .is_some())
            });
    }

    /// Insertion preview: hovering a strip gap, "+", or empty strip
    /// background while dragging a pane paints the slot where the fresh
    /// top-level tab will land; dragging a strip tab paints the slot it
    /// will reorder into. Mirrors the pane drop wash.
    fn paint_strip_drop_preview(&self, ui: &mut egui::Ui, drop: &StripDrop<'_>) {
        if drop.released || !ui.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let pane_gap = drop.frame.drag_same_project
            && self.drag.pane.is_some()
            && Self::strip_interior_tab(drop.strip_rects, pos).is_none();
        let tab_member = self
            .drag
            .tab
            .clone()
            .is_some_and(|dragged| drop.workspace.tabs.iter().any(|tab| tab.id == dragged));
        if !(pane_gap || tab_member) {
            return;
        }
        let Some(index) = drop.insertion_index(pos) else {
            return;
        };
        let accent = appearance::color(&self.theme.accent);
        if drop.plus_rect.contains(pos) {
            ui.painter().rect_stroke(
                drop.plus_rect,
                4,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        Self::paint_strip_insertion(super::super::drag::PaintStripInsertionInput {
            ui,
            strip: &drop.frame.strip_rect,
            tabs: drop.strip_rects,
            index,
            accent,
        });
        if let Some(dragged) = self.drag.tab.clone()
            && let Some((_, rect)) = drop.strip_rects.iter().find(|(id, _)| id == &dragged)
        {
            ui.painter().rect_stroke(
                *rect,
                0,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    }
}
