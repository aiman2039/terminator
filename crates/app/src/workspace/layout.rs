use super::model::{Workspace, WorkspaceTab};
use super::query::{contains_browser, validate_layout};
use crate::Tab;
use anyhow::{Context, Result, ensure};
use egui_dock::DockState;
use std::collections::HashSet;

impl Workspace {
    pub fn from_layout(layout: DockState<Tab>) -> Self {
        let id = terminator_core::id();
        let primary = layout.iter_all_tabs().next().map(|(_, tab)| tab.clone());
        Self {
            version: 2,
            active: id.clone(),
            tabs: vec![WorkspaceTab {
                id,
                primary,
                layout,
            }],
        }
    }

    pub fn empty() -> Self {
        Self::from_layout(DockState::new(vec![]))
    }

    pub fn load(value: serde_json::Value) -> Result<Self> {
        if value.is_null() {
            return Ok(Self::empty());
        }
        if value.get("version").is_some() {
            let mut value = terminator_core::sanitize_layout(value);
            crate::rewrite_html_tabs(&mut value);
            let mut workspace: Self =
                serde_json::from_value(value).context("Invalid project tabs")?;
            ensure!(
                matches!(workspace.version, 2..=7),
                "Unsupported project tab layout version"
            );
            if contains_browser(&workspace) {
                workspace.version = workspace.version.max(6);
            }
            ensure!(!workspace.tabs.is_empty(), "Project tab layout has no tabs");
            let mut ids = std::collections::HashSet::new();
            ensure!(
                workspace
                    .tabs
                    .iter()
                    .all(|tab| !tab.id.is_empty() && ids.insert(&tab.id)),
                "Duplicate or empty project tab ID"
            );
            ensure!(
                workspace.tabs.iter().any(|t| t.id == workspace.active),
                "Active project tab is missing"
            );
            for group in &mut workspace.tabs {
                for (_, pane) in group.layout.iter_all_tabs_mut() {
                    if let Tab::Browser { id, .. } = pane
                        && id.is_empty()
                    {
                        *id = terminator_core::id();
                    }
                }
                if let Some(Tab::Browser { id, target }) = &mut group.primary
                    && id.is_empty()
                {
                    *id = group
                        .layout
                        .iter_all_tabs()
                        .find_map(|(_, pane)| match pane {
                            Tab::Browser {
                                id,
                                target: current,
                            } if current == target => Some(id.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(terminator_core::id);
                }
            }
            for tab in &workspace.tabs {
                validate_layout(&tab.layout)?;
            }
            Ok(workspace)
        } else {
            let layout = serde_json::from_value(terminator_core::sanitize_layout(value))
                .context("Invalid legacy split layout")?;
            validate_layout(&layout)?;
            Ok(Self::from_layout(layout))
        }
    }

    pub fn activate_containing(&mut self, pane: &Tab) -> bool {
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.layout.find_tab(pane).is_some())
        {
            self.active = tab.id.clone();
            true
        } else {
            false
        }
    }

    pub fn add(&mut self, id: String, pane: Tab) {
        self.add_at(self.tabs.len(), id, pane);
    }

    pub fn add_at(&mut self, index: usize, id: String, mut pane: Tab) {
        if let Tab::Browser { id, .. } = &mut pane
            && id.is_empty()
        {
            *id = terminator_core::id();
        }
        self.version = self.version.max(pane.layout_version());
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        let index = index.min(self.tabs.len());
        self.tabs.insert(
            index,
            WorkspaceTab {
                id: id.clone(),
                primary: Some(pane.clone()),
                layout: DockState::new(vec![pane]),
            },
        );
        self.active = id;
        self.main_surface_mut()
            .set_focused_node(egui_dock::NodeIndex::root());
    }

    pub fn close(&mut self, id: &str) {
        let previous = self.active_index();
        self.tabs.retain(|tab| tab.id != id);
        self.normalize(previous);
    }

    fn group_with_pane(&self, pane: &Tab) -> Option<usize> {
        self.tabs
            .iter()
            .position(|group| group.layout.find_tab(pane).is_some())
    }

    fn refresh_primary(&mut self, group: usize, removed: &Tab) {
        let Some(tab) = self.tabs.get_mut(group) else {
            return;
        };
        let stale = tab.primary.as_ref() == Some(removed);
        if stale {
            let next = tab
                .layout
                .iter_all_tabs()
                .next()
                .map(|(_, pane)| pane.clone());
            tab.primary = next;
        }
    }

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
        if src_len == 1 && dst_len == 1 {
            // Exchange single panes across groups: no structure changes, so
            // both terminals stay exactly where the user can see them.
            let Some(other) = other else {
                return false;
            };
            if other == *pane {
                return true;
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
            return true;
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

    pub fn move_pane_to_split(
        &mut self,
        pane: &Tab,
        dest_group: &str,
        dest: egui_dock::NodePath,
        split: egui_dock::Split,
    ) -> bool {
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
        if self.contains(&pane) {
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

    pub(crate) fn drop_empty_tabs(&mut self) {
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub(crate) fn remove_terminal_ids(&mut self, ids: &HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        let hit = self.tabs.iter().any(|tab| {
            tab.layout
                .iter_all_tabs()
                .any(|(_, pane)| matches!(pane, Tab::Terminal(sid) if ids.contains(sid)))
        });
        if !hit {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            let panes: Vec<Tab> = tab
                .layout
                .iter_all_tabs()
                .filter(|(_, pane)| matches!(pane, Tab::Terminal(sid) if ids.contains(sid)))
                .map(|(_, pane)| pane.clone())
                .collect();
            for pane in panes {
                while let Some(path) = tab.layout.find_tab(&pane) {
                    tab.layout.remove_tab(path);
                }
                if tab.primary.as_ref() == Some(&pane) {
                    tab.primary = tab
                        .layout
                        .iter_all_tabs()
                        .next()
                        .map(|(_, pane)| pane.clone());
                }
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub(crate) fn adopt_dock(&mut self, mut layout: DockState<Tab>, activate: bool) -> bool {
        let primary = layout
            .main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| tab.clone())
            .or_else(|| layout.iter_all_tabs().next().map(|(_, tab)| tab.clone()));
        let Some(primary) = primary else {
            return false;
        };
        self.version = layout
            .iter_all_tabs()
            .map(|(_, tab)| tab.layout_version())
            .fold(self.version, u32::max);
        let active_index = self.active_index();
        let reuse = self
            .tabs
            .get(active_index)
            .is_some_and(|tab| tab.layout.iter_all_tabs().next().is_none());
        if reuse {
            let Some(tab) = self.tabs.get_mut(active_index) else {
                return false;
            };
            tab.layout = layout;
            tab.primary = Some(primary);
            return true;
        }
        let id = terminator_core::id();
        self.tabs.push(WorkspaceTab {
            id: id.clone(),
            primary: Some(primary),
            layout,
        });
        if activate {
            self.active = id;
        }
        true
    }

    pub fn strip_player(&mut self) {
        if !self.tabs.iter().any(|tab| {
            tab.primary == Some(Tab::Player) || tab.layout.find_tab(&Tab::Player).is_some()
        }) {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            while let Some(path) = tab.layout.find_tab(&Tab::Player) {
                tab.layout.remove_tab(path);
            }
            if tab.primary == Some(Tab::Player) {
                tab.primary = tab
                    .layout
                    .iter_all_tabs()
                    .next()
                    .map(|(_, pane)| pane.clone());
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub fn remove_session(&mut self, sid: &str) {
        let target = Tab::Terminal(sid.into());
        if !self
            .tabs
            .iter()
            .any(|tab| tab.layout.find_tab(&target).is_some())
        {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            if let Some(path) = tab.layout.find_tab(&Tab::Terminal(sid.into())) {
                tab.layout.remove_tab(path);
                if tab.primary == Some(Tab::Terminal(sid.into())) {
                    tab.primary = tab
                        .layout
                        .iter_all_tabs()
                        .next()
                        .map(|(_, tab)| tab.clone());
                }
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    fn normalize(&mut self, previous: usize) {
        if self.tabs.is_empty() {
            // An emptied workspace is rebuilt from scratch. `Self::empty()` mints
            // a fresh top-level id, so callers that normalize every frame (or on
            // every unrelated mutation) would otherwise see a different layout
            // each time and re-persist it. Keep the previous identity instead.
            let id = std::mem::take(&mut self.active);
            let version = self.version;
            *self = Self::empty();
            self.version = version;
            if !id.is_empty() {
                if let Some(tab) = self.tabs.first_mut() {
                    tab.id.clone_from(&id);
                }
                self.active = id;
            }
        } else if !self.tabs.iter().any(|tab| tab.id == self.active) {
            let fallback = previous
                .saturating_sub(1)
                .min(self.tabs.len().saturating_sub(1));
            if let Some(tab) = self.tabs.get(fallback) {
                self.active.clone_from(&tab.id);
            }
        }
    }
}
