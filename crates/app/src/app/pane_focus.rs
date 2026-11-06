//! Vim window keys from a native editor: Ctrl-W h/j/k/l/w and
//! `:bn` / `:bp` move between panes and the tabs of the focused pane.
use eframe::egui;
use egui_dock::NodeIndex;

use super::super::*;

impl App {
    /// Vim Ctrl-W h/j/k/l: focus the nearest pane on that side of the
    /// focused one (by painted rects); `w` cycles like Next pane. The
    /// keyboard follows into a native editor there.
    pub(crate) fn focus_pane_toward(&mut self, direction: terminator_native_edit::vim::PaneMove) {
        use terminator_native_edit::vim::PaneMove;
        if direction == PaneMove::Next {
            self.focus_next_pane();
        } else {
            let Some(project) = self.selected.clone() else {
                return;
            };
            let Some(dock) = self.layouts.get_mut(&project) else {
                return;
            };
            let tree = dock.main_surface();
            let Some(from) = tree
                .focused_leaf()
                .and_then(|node| tree[node].rect())
                .map(|rect| rect.center())
            else {
                return;
            };
            let toward = |at: egui::Pos2| {
                let delta = egui::vec2(at.x - from.x, at.y - from.y);
                let (along, across) = match direction {
                    PaneMove::Left => (-delta.x, delta.y),
                    PaneMove::Right => (delta.x, delta.y),
                    PaneMove::Up => (-delta.y, delta.x),
                    _ => (delta.y, delta.x),
                };
                (along > 1.0).then_some(along + across.abs() * 2.0)
            };
            let best = tree
                .iter()
                .enumerate()
                .filter(|(_, node)| node.is_leaf())
                .filter_map(|(index, node)| {
                    let score = toward(node.rect()?.center())?;
                    Some((NodeIndex(index), score))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((node, _)) = best else {
                return;
            };
            dock.main_surface_mut().set_focused_node(node);
        }
        self.follow_focused_pane();
    }

    /// Vim `:bn` / `:bp`: the next or previous tab of the focused pane,
    /// wrapping.
    pub(crate) fn cycle_pane_tab(&mut self, forward: bool) {
        let Some(dock) = self
            .selected
            .as_ref()
            .and_then(|id| self.layouts.get_mut(id))
        else {
            return;
        };
        let tree = dock.main_surface_mut();
        let Some(node) = tree.focused_leaf() else {
            return;
        };
        let Some(leaf) = tree[node].get_leaf_mut() else {
            return;
        };
        let count = leaf.tabs.len();
        if count < 2 {
            return;
        }
        let step = if forward { 1 } else { count.saturating_sub(1) };
        leaf.active.0 = leaf
            .active
            .0
            .saturating_add(step)
            .checked_rem(count)
            .unwrap_or(0);
        self.follow_focused_pane();
    }

    /// Move keyboard focus to the active tab of the focused leaf.
    fn follow_focused_pane(&mut self) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(dock) = self.layouts.remove(&project) else {
            return;
        };
        let tree = dock.main_surface();
        let pane = tree
            .focused_leaf()
            .and_then(|node| tree[node].get_leaf())
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .cloned();
        if let Some(pane) = pane {
            self.focus_moved_view(&pane, &project, &dock);
        }
        self.layouts.insert(project, dock);
    }
}
