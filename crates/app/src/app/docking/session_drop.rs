//! A session row dragged from the project sidebar onto a split leaf of the
//! main dock. The middle of a leaf joins it, an edge band opens a split,
//! like a caption drag. A pane already in the dock moves; a background
//! session attaches there.

use super::pane_drop::{PaintLandingPreviewInput, PaneDrop};
use crate::sidebar_ui::TreeDrag;
use crate::*;

impl App {
    /// Preview and complete a sidebar session drop over `dock`, the main
    /// dock of `project`.
    pub(crate) fn finish_session_drop(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        dock: &mut Workspace,
    ) {
        let Some(payload) = egui::DragAndDrop::payload::<TreeDrag>(ui.ctx()) else {
            return;
        };
        let TreeDrag::Session { project: from, sid } = payload.as_ref() else {
            return;
        };
        if from != project || self.is_strip_session(project, sid) || self.is_floated_session(sid) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos())) else {
            return;
        };
        let Some((path, rect)) = dock
            .iter_leaves()
            .find(|(_, leaf)| leaf.rect.contains(pos))
            .map(|(path, leaf)| (path, leaf.rect))
        else {
            return;
        };
        let drop = PaneDrop {
            pane: Tab::Terminal(sid.clone()),
            target: Some((path, rect)),
            tab_insert: None,
            zone: Some(Self::pane_drop_zone(rect, pos)),
        };
        if !ui.input(|i| i.pointer.any_released()) {
            self.paint_landing_preview(PaintLandingPreviewInput {
                ui,
                dock,
                pane: &drop.pane,
                target: Some(path),
                zone: drop.zone,
            });
            ui.ctx().request_repaint();
            return;
        }
        egui::DragAndDrop::clear_payload(ui.ctx());
        if self.land_session_drop(dock, &drop, path) {
            self.pane_drop_landed(dock, &drop.pane);
            self.pending_layout_save = true;
        }
    }

    fn land_session_drop(
        &self,
        dock: &mut Workspace,
        drop: &PaneDrop,
        path: egui_dock::NodePath,
    ) -> bool {
        let group = dock.active.clone();
        if dock.contains(&drop.pane) {
            return drop.move_within(dock, &group, path);
        }
        if dock.leaf(path).is_err() {
            return false;
        }
        let pane = drop.pane.clone();
        let tree = dock.main_surface_mut();
        match drop.zone.and_then(PaneDropZone::split) {
            None => {
                tree.set_focused_node(path.node);
                tree.push_to_focused_leaf(pane);
            }
            Some(egui_dock::Split::Above) => {
                tree.split_above(path.node, 0.5, vec![pane]);
            }
            Some(egui_dock::Split::Below) => {
                tree.split_below(path.node, 0.5, vec![pane]);
            }
            Some(egui_dock::Split::Left) => {
                tree.split_left(path.node, 0.5, vec![pane]);
            }
            Some(egui_dock::Split::Right) => {
                tree.split_right(path.node, 0.5, vec![pane]);
            }
        }
        if let Some(found) = dock.find_tab(&drop.pane) {
            let _ = dock.set_active_tab(found);
            dock.set_focused_node_and_surface(found.node_path());
        }
        true
    }
}
