use super::*;

impl Workspace {
    pub fn move_pane_to_group(&mut self, pane: &Tab, dest_group: &str) -> bool {
        let Some(dst) = self.tabs.iter().position(|tab| tab.id == dest_group) else {
            return false;
        };
        let Some(tab) = self.tabs.get(dst) else {
            return false;
        };
        let path = tab
            .layout
            .main_surface()
            .focused_leaf()
            .map(|node| egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            })
            .or_else(|| tab.layout.iter_leaves().next().map(|(path, _)| path));
        let Some(path) = path else {
            return false;
        };
        self.move_pane_to_group_leaf(pane, dest_group, path)
    }

    pub fn move_pane_to_leaf(&mut self, pane: &Tab, dest: egui_dock::NodePath) -> bool {
        let active = self.active_index();
        let Some(src) = self
            .tabs
            .get(active)
            .and_then(|tab| tab.layout.find_tab(pane))
        else {
            return false;
        };
        if self
            .tabs
            .get(active)
            .is_none_or(|tab| tab.layout.leaf(dest).is_err())
        {
            return false;
        }
        if src.node_path() == dest {
            let Some(layout) = self.tabs.get_mut(active).map(|tab| &mut tab.layout) else {
                return false;
            };
            let _ = layout.set_active_tab(src);
            layout.set_focused_node_and_surface(dest);
            return true;
        }
        let (src_len, dst_len) = match (
            self.tabs
                .get(active)
                .and_then(|tab| tab.layout.leaf(src.node_path()).ok()),
            self.tabs
                .get(active)
                .and_then(|tab| tab.layout.leaf(dest).ok()),
        ) {
            (Some(src_leaf), Some(dst_leaf)) => (src_leaf.tabs.len(), dst_leaf.tabs.len()),
            _ => return false,
        };
        if src_len == 1 && dst_len == 1 {
            let Some(layout) = self.tabs.get_mut(active).map(|tab| &mut tab.layout) else {
                return false;
            };
            let src_node = src.node_path();
            let other = layout
                .leaf(dest)
                .ok()
                .and_then(|leaf| leaf.tabs.first().cloned());
            let Some(other) = other else {
                return false;
            };
            if other == *pane {
                return true;
            }
            if let Ok(leaf) = layout.leaf_mut(src_node)
                && let Some(slot) = leaf.tabs.get_mut(0)
            {
                *slot = other;
            }
            if let Ok(leaf) = layout.leaf_mut(dest)
                && let Some(slot) = leaf.tabs.get_mut(0)
            {
                *slot = pane.clone();
            }
            let _ = layout.set_active_tab(layout.find_tab(pane).unwrap_or(src));
            layout.set_focused_node_and_surface(dest);
            return true;
        }
        {
            let Some(layout) = self.tabs.get_mut(active).map(|tab| &mut tab.layout) else {
                return false;
            };
            layout.move_tab(src, (dest, egui_dock::TabInsert::Append));
            if let Some(path) = layout.find_tab(pane) {
                let _ = layout.set_active_tab(path);
                layout.set_focused_node_and_surface(path.node_path());
            }
        }
        true
    }

    pub fn move_pane_to_group_leaf(
        &mut self,
        pane: &Tab,
        dest_group: &str,
        dest: egui_dock::NodePath,
    ) -> bool {
        let Some(src) = self.group_with_pane(pane) else {
            return false;
        };
        let Some(dst) = self.tabs.iter().position(|tab| tab.id == dest_group) else {
            return false;
        };
        if src == dst {
            return self.move_pane_to_leaf(pane, dest);
        }
        let (src_path, src_len, dst_len, other) = {
            let Some(dst_tab) = self.tabs.get(dst) else {
                return false;
            };
            let Ok(dst_leaf) = dst_tab.layout.leaf(dest) else {
                return false;
            };
            let Some(src_tab) = self.tabs.get(src) else {
                return false;
            };
            let Some(src_path) = src_tab.layout.find_tab(pane) else {
                return false;
            };
            let Ok(src_leaf) = src_tab.layout.leaf(src_path.node_path()) else {
                return false;
            };
            (
                src_path,
                src_leaf.tabs.len(),
                dst_leaf.tabs.len(),
                dst_leaf.tabs.first().cloned(),
            )
        };
        if let Some(result) = self.exchange_single_pane(ExchangeSinglePaneInput {
            src,
            dst,
            pane,
            dest,
            src_path,
            dest_group,
            other,
            src_len,
            dst_len,
        }) {
            return result;
        }
        if let Some(tab) = self.tabs.get_mut(src) {
            let removed = tab.layout.remove_tab(src_path);
            debug_assert!(removed.is_some());
        }
        self.refresh_primary(src, pane);
        if let Some(tab) = self.tabs.get_mut(dst)
            && let Ok(leaf) = tab.layout.leaf_mut(dest)
        {
            leaf.append_tab(pane.clone());
        }
        if let Some(tab) = self.tabs.get_mut(dst)
            && let Some(path) = tab.layout.find_tab(pane)
        {
            let _ = tab.layout.set_active_tab(path);
            tab.layout.set_focused_node_and_surface(path.node_path());
        }
        self.version = self.version.max(pane.layout_version());
        dest_group.clone_into(&mut self.active);
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
        true
    }

    fn exchange_single_pane(&mut self, input: ExchangeSinglePaneInput<'_>) -> Option<bool> {
        let ExchangeSinglePaneInput {
            src,
            dst,
            pane,
            dest,
            src_path,
            dest_group,
            other,
            src_len,
            dst_len,
        } = input;
        if src_len == 1 && dst_len == 1 {
            // Exchange single panes across groups: no structure changes, so
            // both terminals stay exactly where the user can see them.
            let Some(other) = other else {
                return Some(false);
            };
            if other == *pane {
                return Some(true);
            }
            if let Some(tab) = self.tabs.get_mut(src)
                && let Ok(leaf) = tab.layout.leaf_mut(src_path.node_path())
                && let Some(slot) = leaf.tabs.get_mut(0)
            {
                *slot = other;
            }
            if let Some(tab) = self.tabs.get_mut(dst)
                && let Ok(leaf) = tab.layout.leaf_mut(dest)
                && let Some(slot) = leaf.tabs.get_mut(0)
            {
                *slot = pane.clone();
            }
            if let Some(tab) = self.tabs.get_mut(dst)
                && let Some(path) = tab.layout.find_tab(pane)
            {
                let _ = tab.layout.set_active_tab(path);
                tab.layout.set_focused_node_and_surface(path.node_path());
            }
            dest_group.clone_into(&mut self.active);
            return Some(true);
        }
        None
    }

    pub fn move_pane_to_split(&mut self, input: MovePaneToSplitInput<'_>) -> bool {
        let MovePaneToSplitInput {
            pane,
            dest_group,
            dest,
            split,
        } = input;

        let Some(src) = self.group_with_pane(pane) else {
            return false;
        };
        let Some(dst) = self.tabs.iter().position(|tab| tab.id == dest_group) else {
            return false;
        };
        if self
            .tabs
            .get(dst)
            .is_none_or(|tab| tab.layout.leaf(dest).is_err())
        {
            return false;
        }
        if src == dst {
            let Some(src_path) = self.tabs.get(src).and_then(|tab| tab.layout.find_tab(pane))
            else {
                return false;
            };
            let Some(tab) = self.tabs.get_mut(src) else {
                return false;
            };
            tab.layout
                .move_tab(src_path, (dest, egui_dock::TabInsert::Split(split)));
            if let Some(path) = tab.layout.find_tab(pane) {
                let _ = tab.layout.set_active_tab(path);
                tab.layout.set_focused_node_and_surface(path.node_path());
            }
            return true;
        }
        let Some(src_path) = self.tabs.get(src).and_then(|tab| tab.layout.find_tab(pane)) else {
            return false;
        };
        if let Some(tab) = self.tabs.get_mut(src) {
            let removed = tab.layout.remove_tab(src_path);
            debug_assert!(removed.is_some());
        }
        self.refresh_primary(src, pane);
        if let Some(tab) = self.tabs.get_mut(dst) {
            let tree = tab.layout.main_surface_mut();
            match split {
                egui_dock::Split::Above => tree.split_above(dest.node, 0.5, vec![pane.clone()]),
                egui_dock::Split::Below => tree.split_below(dest.node, 0.5, vec![pane.clone()]),
                egui_dock::Split::Left => tree.split_left(dest.node, 0.5, vec![pane.clone()]),
                egui_dock::Split::Right => tree.split_right(dest.node, 0.5, vec![pane.clone()]),
            };
        }
        if let Some(tab) = self.tabs.get_mut(dst)
            && let Some(path) = tab.layout.find_tab(pane)
        {
            let _ = tab.layout.set_active_tab(path);
            tab.layout.set_focused_node_and_surface(path.node_path());
        }
        self.version = self.version.max(pane.layout_version());
        dest_group.clone_into(&mut self.active);
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
        true
    }

    pub(crate) fn take_pane(&mut self, pane: &Tab) -> Option<String> {
        let src = self.group_with_pane(pane)?;
        let home = self.tabs.get(src)?.id.clone();
        let path = self.tabs.get(src)?.layout.find_tab(pane)?;
        self.tabs.get_mut(src)?.layout.remove_tab(path);
        self.refresh_primary(src, pane);
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
        Some(home)
    }

    pub(crate) fn dock_back(&mut self, pane: Tab, home: &str) {
        if !matches!(pane, Tab::NativeEditor { .. }) && self.contains(&pane) {
            self.activate_containing(&pane);
            return;
        }
        self.version = self.version.max(pane.layout_version());
        if self.tabs.iter().any(|tab| tab.id == home) {
            home.clone_into(&mut self.active);
        }
        self.push_to_focused_leaf(pane);
    }

    pub fn move_pane_to_new_group_at(&mut self, pane: &Tab, index: usize) -> Option<String> {
        let src = self.group_with_pane(pane)?;
        let src_id = self.tabs.get(src)?.id.clone();
        let path = self.tabs.get(src)?.layout.find_tab(pane)?;
        self.tabs.get_mut(src)?.layout.remove_tab(path);
        self.refresh_primary(src, pane);
        let id = terminator_core::id();
        self.version = self.version.max(pane.layout_version());
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
        let mut index = index;
        if !self.tabs.iter().any(|tab| tab.id == src_id) && src < index {
            index = index.saturating_sub(1);
        }
        let index = index.min(self.tabs.len());
        self.add_at(index, id.clone(), pane.clone());
        Some(id)
    }

    pub fn reorder_group(&mut self, group_id: &str, index: usize) -> bool {
        let Some(from) = self.tabs.iter().position(|tab| tab.id == group_id) else {
            return false;
        };
        let index = index.min(self.tabs.len().saturating_sub(1));
        if from == index {
            return true;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(index, tab);
        true
    }
}
