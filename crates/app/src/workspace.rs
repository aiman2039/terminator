//! Project-level tabs; each owns an independent split tree and pane focus.
use crate::Tab;
use anyhow::{Context, Result, ensure};
use egui_dock::DockState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct WorkspaceTab {
    pub id: String,
    pub primary: Option<Tab>,
    pub layout: DockState<Tab>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Workspace {
    pub version: u32,
    pub active: String,
    pub tabs: Vec<WorkspaceTab>,
}
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
    pub fn active_pane(&self) -> Option<&Tab> {
        self.main_surface()
            .focused_leaf()
            // Checked: an emptied tree keeps stale focus, and blind indexing
            // would panic.
            .and_then(|node| self.main_surface().leaf(node).ok())
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .or_else(|| self.iter_all_tabs().next().map(|(_, tab)| tab))
    }
    pub fn active_index(&self) -> usize {
        self.tabs
            .iter()
            .position(|tab| tab.id == self.active)
            .unwrap_or(0)
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
    pub fn contains(&self, pane: &Tab) -> bool {
        self.tabs.iter().any(|tab| {
            tab.layout.iter_all_tabs().any(|(_, existing)| match pane {
                Tab::Browser { id, target } if id.is_empty() => {
                    matches!(existing, Tab::Browser { target: current, .. } if current == target)
                }
                _ => existing == pane,
            })
        })
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
    pub fn ids(&self) -> Vec<String> {
        self.tabs.iter().map(|tab| tab.id.clone()).collect()
    }
    pub fn ids_before(&self, id: &str) -> Vec<String> {
        self.ids().into_iter().take_while(|tab| tab != id).collect()
    }
    pub fn ids_after(&self, id: &str) -> Vec<String> {
        self.ids()
            .into_iter()
            .skip_while(|tab| tab != id)
            .skip(1)
            .collect()
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

    /// Move a pane between top-level tabs of the same project, landing on
    /// the destination's focused split. Single panes exchange places so both
    /// stay visible; otherwise the pane joins the leaf. The destination group
    /// becomes active with the moved pane focused. Returns false when the
    /// pane or destination is missing, or when both already match.
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

    /// Move a pane within the active top-level tab. Dropping onto its own
    /// leaf focuses it; dropping single-pane leaves onto each other swaps
    /// their positions so split layouts visibly rearrange; otherwise the
    /// pane appends to the destination leaf.
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

    /// Move a pane into a specific split leaf of another top-level tab,
    /// which becomes active with the moved pane focused. When both leaves
    /// hold a single pane the two exchange places so nothing disappears
    /// behind a hidden tab stack; otherwise the pane joins the leaf and
    /// emptied source groups are dropped. Within the same group this behaves
    /// like [`Self::move_pane_to_leaf`]. Returns false when the pane,
    /// destination group, or destination leaf is missing.
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

    /// Drop a pane onto an edge of a split leaf, opening it in a new split
    /// beside that leaf (above, below, left, or right). Works within one tab
    /// and across tabs; the destination group becomes active with the moved
    /// pane focused. Dropping a lone pane onto an edge of its own leaf is a
    /// no-op beyond focusing it: one terminal cannot fill two splits.
    /// Returns false when the pane, destination group, or leaf is missing.
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

    /// Remove a pane for floating outside the dock in its own window,
    /// dropping an emptied top-level tab. Returns the originating group id
    /// so closing the window can dock the pane back. Returns None when the
    /// pane is not in this workspace.
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

    /// Dock a formerly floating pane back: into its home group when that
    /// still exists, else into the active tab's focused leaf. Never opens
    /// a new top-level tab; readers tolerate a stale `primary`.
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

    /// Move a pane out of its group into a fresh top-level tab at `index`,
    /// which becomes active. Dropping a pane between two strip tabs lands
    /// the new tab exactly there; an emptied source group is dropped first,
    /// shifting later slots down by one. Returns the new group id, or None
    /// when missing.
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

    /// Reorder a top-level tab, moving it to `index`. The active tab id is
    /// untouched, so focus follows the tab, not the slot. Returns false
    /// when the group is missing.
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

    /// Drop top-level tabs left without panes and restore the non-empty
    /// invariant the [`Deref`] impls rely on. Pane-level removers that
    /// retain directly (native closes) must call this instead of retaining
    /// inline, or the next dock access panics on `tabs[0]`.
    pub(crate) fn drop_empty_tabs(&mut self) {
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    /// Drop shell panes whose ids are in `ids`. Emptied top-level tabs go away.
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

    /// Install `layout` as a top-level tab. An empty active tab is reused so a
    /// blank workspace does not keep a second empty group. `activate` selects
    /// the new tab; a reused blank tab is already the one on screen.
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
// Serde restores raw docking indices without the checks used by UI setters.
// Reject invalid persisted focus before any App or renderer indexing occurs.
fn contains_browser(workspace: &Workspace) -> bool {
    workspace.tabs.iter().any(|tab| {
        matches!(tab.primary, Some(Tab::Browser { .. }))
            || tab
                .layout
                .iter_all_tabs()
                .any(|(_, pane)| matches!(pane, Tab::Browser { .. }))
    })
}

pub(crate) fn validate_layout(layout: &DockState<Tab>) -> Result<()> {
    ensure!(
        matches!(
            layout.get_surface(egui_dock::SurfaceIndex::main()),
            Some(egui_dock::Surface::Main(_))
        ),
        "Saved layout has no main surface"
    );
    for surface in layout.iter_surfaces() {
        if let Some(tree) = surface.node_tree()
            && let Some(focus) = tree.focused_leaf()
        {
            ensure!(
                tree.iter()
                    .nth(focus.0)
                    .is_some_and(egui_dock::Node::is_leaf),
                "Invalid saved pane focus"
            );
        }
    }
    Ok(())
}

// Existing pane operations intentionally address the active top-level tab only.
impl Deref for Workspace {
    type Target = DockState<Tab>;
    fn deref(&self) -> &Self::Target {
        let index = self.active_index();
        if let Some(tab) = self.tabs.get(index).or_else(|| self.tabs.first()) {
            return &tab.layout;
        }
        static FALLBACK: std::sync::OnceLock<DockState<Tab>> = std::sync::OnceLock::new();
        FALLBACK.get_or_init(|| DockState::new(Vec::new()))
    }
}
impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if self.tabs.is_empty() {
            let id = if self.active.is_empty() {
                terminator_core::id()
            } else {
                self.active.clone()
            };
            self.active.clone_from(&id);
            self.tabs.push(WorkspaceTab {
                id,
                primary: None,
                layout: DockState::new(Vec::new()),
            });
        }
        let preferred = self.active_index();
        let index = if self.tabs.get(preferred).is_some() {
            preferred
        } else {
            0
        };
        if let Some(tab) = self.tabs.get_mut(index) {
            &mut tab.layout
        } else {
            Box::leak(Box::new(DockState::new(Vec::new())))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_browser_ids_migrate_once_and_primary_matches_the_pane() {
        let mut workspace = Workspace::empty();
        workspace.add("browser".into(), Tab::browser_file("/tmp/page.html".into()));
        let mut value = terminator_core::sanitize_layout(serde_json::to_value(workspace).unwrap());
        fn remove_browser_ids(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(browser) = map
                        .get_mut("Browser")
                        .and_then(serde_json::Value::as_object_mut)
                    {
                        browser.remove("id");
                    }
                    for nested in map.values_mut() {
                        remove_browser_ids(nested);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        remove_browser_ids(value);
                    }
                }
                _ => {}
            }
        }
        remove_browser_ids(&mut value);
        let loaded = Workspace::load(value).unwrap();
        let key = loaded.active_pane().unwrap().key();
        assert_eq!(loaded.tabs[0].primary.as_ref().unwrap().key(), key);
        let restored = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&loaded).unwrap(),
        ))
        .unwrap();
        assert_eq!(restored.active_pane().unwrap().key(), key);
    }

    #[test]
    fn emptying_every_pane_keeps_dock_access_usable() {
        // Regression: `:qa` closing the last pane left `tabs` empty, and
        // the next dock access panicked on `tabs[0]` in the Deref impls.
        let pane = Tab::Terminal("qa-last-pane".to_string());
        let mut workspace = Workspace::from_layout(DockState::new(vec![pane.clone()]));
        let path = workspace.tabs[0]
            .layout
            .find_tab(&pane)
            .expect("pane present");
        workspace.tabs[0].layout.remove_tab(path);
        workspace.drop_empty_tabs();
        assert_eq!(workspace.tabs.len(), 1);
        // Read access through Deref.
        assert!(workspace.iter_all_tabs().next().is_none());
        // Write access through DerefMut (the exact panic site).
        workspace.add("qa-next".to_string(), Tab::Terminal("qa-next".to_string()));
        assert!(workspace.contains(&Tab::Terminal("qa-next".to_string())));
    }

    #[test]
    fn missing_main_surface_is_rejected_before_pane_access() {
        let mut saved =
            terminator_core::sanitize_layout(serde_json::to_value(Workspace::empty()).unwrap());
        saved["tabs"][0]["layout"]["surfaces"] = serde_json::json!([]);
        let result = Workspace::load(saved);
        if let Ok(workspace) = &result {
            workspace.active_pane();
        }
        assert!(result.is_err());
    }

    #[test]
    fn invalid_focus_is_rejected_in_legacy_and_versioned_layouts() {
        let dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        let mut legacy = terminator_core::sanitize_layout(serde_json::to_value(&dock).unwrap());
        legacy["surfaces"][0]["Main"]["focused_node"] = serde_json::json!(999);
        assert!(
            Workspace::load(legacy.clone())
                .unwrap_err()
                .to_string()
                .contains("focus")
        );
        for version in [2, 3, 4, 5, 6, 7] {
            let mut saved = terminator_core::sanitize_layout(
                serde_json::to_value(Workspace::from_layout(dock.clone())).unwrap(),
            );
            saved["version"] = serde_json::json!(version);
            saved["tabs"][0]["layout"] = legacy.clone();
            assert!(
                Workspace::load(saved)
                    .unwrap_err()
                    .to_string()
                    .contains("focus")
            );
        }
    }

    #[test]
    fn media_layout_version_round_trips_and_preserves_legacy_tabs() {
        let mut workspace = Workspace::empty();
        workspace.add("shell".into(), Tab::Terminal("shell".into()));
        assert_eq!(workspace.version, 2);
        workspace.add(
            "image".into(),
            Tab::Image {
                path: "/image.png".into(),
            },
        );
        assert_eq!(workspace.version, 3);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Terminal("shell".into())));
        assert!(restored.contains(&Tab::Image {
            path: "/image.png".into()
        }));
        workspace.add("html".into(), Tab::browser_file("/page.html".into()));
        assert_eq!(workspace.version, 6);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::browser_file("/page.html".into())));
        workspace.add("player".into(), Tab::Player);
        assert_eq!(workspace.version, 6);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Player));
        workspace.add(
            "native".into(),
            Tab::NativeEditor {
                path: "/notes/todo.md".into(),
            },
        );
        assert_eq!(workspace.version, 7);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::NativeEditor {
            path: "/notes/todo.md".into()
        }));
    }

    #[test]
    fn v4_html_tabs_migrate_to_browser() {
        let mut workspace = Workspace::empty();
        workspace.add("html".into(), Tab::browser_file("/page.html".into()));
        let mut saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        saved["version"] = serde_json::json!(4);
        demote_browser_tabs_to_html(&mut saved);
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::browser_file("/page.html".into())));
        assert_eq!(restored.version, 6);
    }

    #[test]
    fn unsupported_layout_version_is_rejected() {
        let mut saved =
            terminator_core::sanitize_layout(serde_json::to_value(Workspace::empty()).unwrap());
        saved["version"] = serde_json::json!(8);
        assert!(
            Workspace::load(saved)
                .unwrap_err()
                .to_string()
                .contains("Unsupported")
        );
    }

    #[test]
    fn player_only_layout_stays_version_five() {
        let mut workspace = Workspace::empty();
        workspace.add("player".into(), Tab::Player);
        assert_eq!(workspace.version, 5);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert_eq!(restored.version, 5);
        assert!(restored.contains(&Tab::Player));
    }

    #[test]
    fn strip_player_removes_player_panes_and_keeps_other_tabs() {
        let mut workspace = Workspace::empty();
        workspace.add("shell".into(), Tab::Terminal("s".into()));
        workspace.add("player".into(), Tab::Player);
        workspace.strip_player();
        assert!(!workspace.contains(&Tab::Player));
        assert!(workspace.contains(&Tab::Terminal("s".into())));
    }

    #[test]
    fn strip_player_keeps_empty_workspace_identity() {
        let mut workspace = Workspace::empty();
        let before = serde_json::to_value(&workspace).unwrap();
        workspace.strip_player();
        workspace.remove_session("missing");
        assert_eq!(serde_json::to_value(&workspace).unwrap(), before);
    }

    #[test]
    fn emptied_workspace_keeps_its_identity() {
        let mut workspace = Workspace::empty();
        let id = workspace.active.clone();
        workspace.drop_empty_tabs();
        workspace.drop_empty_tabs();
        assert_eq!(workspace.active, id);
        assert_eq!(workspace.tabs[0].id, id);
    }

    fn demote_browser_tabs_to_html(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(browser) = map.remove("Browser") {
                    let path = browser
                        .get("target")
                        .and_then(|target| target.get("File"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    map.insert("Html".into(), serde_json::json!({ "path": path }));
                }
                for nested in map.values_mut() {
                    demote_browser_tabs_to_html(nested);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    demote_browser_tabs_to_html(nested);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn active_pane_is_none_for_an_emptied_dock() {
        let mut dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        let path = dock.find_tab(&Tab::Terminal("shell".into())).unwrap();
        dock.set_focused_node_and_surface(path.node_path());
        dock.remove_tab(path);
        let workspace = Workspace::from_layout(dock);
        assert!(workspace.active_pane().is_none());
    }

    #[test]
    fn legacy_splits_migrate_without_losing_sessions() {
        let mut dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        dock.main_surface_mut().split_below(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("lower".into())],
        );
        let migrated = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&dock).unwrap(),
        ))
        .unwrap();
        assert_eq!(migrated.tabs.len(), 1);
        assert_eq!(migrated.iter_all_tabs().count(), 2);
        assert_eq!(
            migrated.main_surface().focused_leaf(),
            dock.main_surface().focused_leaf()
        );
    }
    #[test]
    fn top_level_tabs_keep_independent_splits_and_survive_restart() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("shell".into())]));
        let original = workspace.active.clone();
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.4,
            vec![Tab::Terminal("split".into())],
        );
        workspace.add("file-tab".into(), Tab::Terminal("editor".into()));
        assert_eq!(workspace.iter_all_tabs().count(), 1);
        assert!(workspace.activate_containing(&Tab::Terminal("split".into())));
        assert_eq!(workspace.active, original);
        assert_eq!(workspace.iter_all_tabs().count(), 2);
        let restored = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&workspace).unwrap(),
        ))
        .unwrap();
        assert_eq!(restored.active, original);
        assert_eq!(restored.tabs.len(), 2);
        assert!(restored.contains(&Tab::Terminal("editor".into())));
    }
    #[test]
    fn add_at_inserts_between_existing_tabs_and_clamps() {
        let mut workspace = Workspace::empty();
        workspace.add("a".into(), Tab::Terminal("a".into()));
        workspace.add("c".into(), Tab::Terminal("c".into()));
        workspace.add_at(1, "b".into(), Tab::Terminal("b".into()));
        assert_eq!(
            workspace
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(workspace.active, "b");
        assert_eq!(workspace.ids(), vec!["a", "b", "c"]);
        assert_eq!(workspace.ids_before("b"), vec!["a"]);
        assert_eq!(workspace.ids_after("b"), vec!["c"]);
        workspace.add_at(99, "d".into(), Tab::Terminal("d".into()));
        assert_eq!(workspace.tabs.last().map(|tab| tab.id.as_str()), Some("d"));
        assert_eq!(workspace.ids_before("a"), Vec::<String>::new());
        assert_eq!(workspace.ids_after("d"), Vec::<String>::new());
    }

    #[test]
    fn closing_one_tab_keeps_other_layouts_and_focus() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("shell".into())]));
        let original = workspace.active.clone();
        workspace.add("file".into(), Tab::Terminal("editor".into()));
        workspace.remove_session("editor");
        assert_eq!(workspace.active, original);
        assert_eq!(workspace.iter_all_tabs().count(), 1);
        workspace.close(&original);
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.iter_all_tabs().count(), 0);
    }

    #[test]
    fn move_pane_to_group_leaf_swaps_single_panes_across_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("second".into(), Tab::Terminal("two".into()));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        // Workspace Deref addresses the active ("second") group; its leaves
        // are the drop targets.
        let leaves: Vec<egui_dock::NodePath> =
            workspace.iter_leaves().map(|(path, _)| path).collect();
        assert_eq!(leaves.len(), 2);
        let target = workspace
            .find_tab(&Tab::Terminal("three".into()))
            .unwrap()
            .node_path();
        // Both leaves hold a single pane, so the terminals exchange places
        // and both tabs survive with everything visible.
        assert!(workspace.move_pane_to_group_leaf(&Tab::Terminal("one".into()), "second", target));
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.tabs.len(), 2);
        let landed = workspace
            .find_tab(&Tab::Terminal("one".into()))
            .unwrap()
            .node_path();
        assert_eq!(landed, target);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
        let first_group = workspace
            .tabs
            .iter()
            .find(|tab| tab.id != workspace.active)
            .expect("source tab survives the swap");
        let swapped_home = first_group
            .layout
            .find_tab(&Tab::Terminal("three".into()))
            .expect("swapped pane stays in the source tab");
        assert_ne!(swapped_home.node_path(), target);
        assert!(!workspace.move_pane_to_group_leaf(
            &Tab::Terminal("ghost".into()),
            "second",
            target
        ));
        assert!(!workspace.move_pane_to_group_leaf(
            &Tab::Terminal("two".into()),
            "missing",
            target
        ));
    }

    #[test]
    fn move_pane_to_group_leaf_joins_stacked_leaves_and_drops_emptied_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        // Destination leaf already stacks two panes: the move joins it and
        // the emptied source tab is dropped.
        workspace.add("second".into(), Tab::Terminal("two".into()));
        workspace.tabs[1].layout = DockState::new(vec![
            Tab::Terminal("two".into()),
            Tab::Terminal("three".into()),
        ]);
        workspace.tabs[1].primary = Some(Tab::Terminal("two".into()));
        let target = workspace.tabs[1]
            .layout
            .find_tab(&Tab::Terminal("two".into()))
            .unwrap()
            .node_path();
        assert!(workspace.move_pane_to_group_leaf(&Tab::Terminal("one".into()), "second", target));
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, "second");
        let leaf = workspace.leaf(target).unwrap();
        assert_eq!(leaf.tabs.len(), 3);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
    }

    #[test]
    fn move_pane_to_split_opens_an_edge_split() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        let target = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        let group = workspace.active.clone();
        assert!(workspace.move_pane_to_split(
            &Tab::Terminal("left".into()),
            group.as_str(),
            target,
            egui_dock::Split::Right,
        ));
        // The emptied source leaf collapses: right splits into right+left
        // in separate leaves with the moved pane focused. (Leaf rectangles
        // only exist after rendering; geometric order is covered by the
        // headless UI drop test below.)
        assert_eq!(workspace.iter_leaves().count(), 2);
        let left_home = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        let right_home = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        assert_ne!(left_home, right_home);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("left".into())));
    }

    #[test]
    fn move_pane_to_split_across_tabs_drops_the_emptied_tab() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("second".into(), Tab::Terminal("two".into()));
        let target = workspace
            .find_tab(&Tab::Terminal("two".into()))
            .unwrap()
            .node_path();
        assert!(workspace.move_pane_to_split(
            &Tab::Terminal("one".into()),
            "second",
            target,
            egui_dock::Split::Below,
        ));
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.iter_leaves().count(), 2);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(!workspace.move_pane_to_split(
            &Tab::Terminal("ghost".into()),
            "second",
            target,
            egui_dock::Split::Below,
        ));
    }

    #[test]
    fn move_pane_to_group_swaps_single_terminals_and_activates_destination() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        let first = workspace.active.clone();
        workspace.add("second".into(), Tab::Terminal("two".into()));
        // Both sides hold one terminal: they exchange places so both tabs
        // survive with everything visible.
        assert!(workspace.move_pane_to_group(&Tab::Terminal("one".into()), "second"));
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.tabs.len(), 2);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(workspace.contains(&Tab::Terminal("two".into())));
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
        let origin = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == first)
            .expect("origin tab survives the swap");
        assert!(
            origin
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(!workspace.move_pane_to_group(&Tab::Terminal("ghost".into()), "second"));
        assert!(!workspace.move_pane_to_group(&Tab::Terminal("one".into()), "missing"));
    }

    #[test]
    fn move_pane_to_leaf_swaps_single_pane_splits() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        let leaves: Vec<egui_dock::NodePath> =
            workspace.iter_leaves().map(|(path, _)| path).collect();
        assert_eq!(leaves.len(), 2);
        let src = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        let dst = leaves.iter().copied().find(|path| *path != src).unwrap();
        assert!(workspace.move_pane_to_leaf(&Tab::Terminal("left".into()), dst));
        assert_eq!(workspace.iter_all_tabs().count(), 2);
        // Positions swapped: "left" now lives where "right" was.
        let now = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        assert_eq!(now, dst);
        let other = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        assert_eq!(other, src);
    }

    #[test]
    fn move_pane_to_new_group_at_end_creates_top_level_tab() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("two".into())],
        );
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("two".into()), workspace.tabs.len())
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(workspace.contains(&Tab::Terminal("two".into())));
        assert!(
            workspace
                .move_pane_to_new_group_at(&Tab::Terminal("ghost".into()), 0)
                .is_none()
        );
    }

    #[test]
    fn move_pane_to_new_group_at_lands_between_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // Dropping "two" at the front removes its group; the new tab
        // lands first with "one" behind it.
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("two".into()), 0)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert!(
            workspace
                .move_pane_to_new_group_at(&Tab::Terminal("ghost".into()), 0)
                .is_none()
        );
    }

    #[test]
    fn move_pane_to_new_group_at_keeps_trailing_slot_when_source_empties() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // Dropping "one" past the end removes its group; the trailing slot
        // shifts down so the new tab still lands last.
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("one".into()), 2)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert_eq!(workspace.active, id);
    }

    #[test]
    fn move_pane_to_new_group_at_keeps_source_slot_when_source_survives() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // "three" leaves its group behind, so no slot shifts: the new tab
        // lands at the end and the source group keeps its panes.
        workspace
            .move_pane_to_new_group_at(&Tab::Terminal("three".into()), 3)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 3);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[2]
                .layout
                .find_tab(&Tab::Terminal("three".into()))
                .is_some()
        );
    }

    #[test]
    fn reorder_group_moves_top_level_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        let first = workspace.tabs[0].id.clone();
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        workspace.add("tC".into(), Tab::Terminal("three".into()));
        assert!(workspace.reorder_group("tC", 0));
        assert_eq!(
            workspace.ids(),
            vec!["tC".to_owned(), first.clone(), "tB".to_owned()]
        );
        // Focus follows the tab, not the slot.
        assert_eq!(workspace.active, "tC");
        assert!(workspace.reorder_group(&first, 2));
        assert_eq!(
            workspace.ids(),
            vec!["tC".to_owned(), "tB".to_owned(), first.clone()]
        );
        assert!(!workspace.reorder_group("ghost", 0));
    }

    #[test]
    fn adopt_dock_reuses_a_blank_tab_and_appends_beside_content() {
        let mut workspace = Workspace::empty();
        let layout = DockState::new(vec![Tab::Terminal("shell".into())]);
        assert!(workspace.adopt_dock(layout, true));
        assert_eq!(workspace.tabs.len(), 1);
        assert!(workspace.contains(&Tab::Terminal("shell".into())));

        workspace.add("editor".into(), Tab::Terminal("edit".into()));
        let editor = workspace.active.clone();
        assert!(workspace.adopt_dock(DockState::new(vec![Tab::Terminal("other".into())]), false));
        assert_eq!(workspace.tabs.len(), 3);
        assert_eq!(workspace.active, editor);
        assert!(workspace.contains(&Tab::Terminal("other".into())));

        let mut ids = HashSet::new();
        ids.insert("shell".into());
        ids.insert("missing".into());
        workspace.remove_terminal_ids(&ids);
        assert!(!workspace.contains(&Tab::Terminal("shell".into())));
        assert!(workspace.contains(&Tab::Terminal("edit".into())));
        assert!(workspace.contains(&Tab::Terminal("other".into())));
    }

    #[test]
    fn detach_file_window_moves_pane_to_fresh_top_level_tab() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        // Detach tears the file window off into a new tab at the end of
        // the strip; the source tab keeps the remaining terminal.
        let id = workspace
            .move_pane_to_new_group_at(&editor, workspace.tabs.len())
            .expect("detach creates a tab");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(workspace.tabs[0].layout.find_tab(&shell).is_some());
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_none());
        assert!(workspace.tabs[1].layout.find_tab(&editor).is_some());
        assert!(
            !workspace
                .move_pane_to_new_group_at(
                    &Tab::NativeEditor {
                        path: "/tmp/ghost.md".into()
                    },
                    0
                )
                .is_some()
        );
    }

    #[test]
    fn take_pane_for_float_drops_emptied_tab_and_reports_home() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        // A lone pane's tab goes away; the home id survives for dock-back.
        workspace.add("lone".into(), Tab::Terminal("solo".into()));
        let home = workspace
            .take_pane(&Tab::Terminal("solo".into()))
            .expect("pane present");
        assert_eq!(home, "lone");
        assert!(workspace.tabs.iter().all(|tab| tab.id != "lone"));
        // Taking from a shared leaf keeps the tab with the other pane.
        let home = workspace.take_pane(&editor).expect("pane present");
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&shell).is_some());
        assert!(workspace.take_pane(&editor).is_none());
    }

    #[test]
    fn take_pane_of_last_pane_keeps_one_empty_group_for_dock_access() {
        let shell = Tab::Terminal("one".into());
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        let home = workspace.take_pane(&shell).expect("pane present");
        assert_eq!(home, workspace.active);
        // Not a new tab: the emptied group is replaced by the single empty
        // group the dock accessors require, holding zero panes. The pane
        // itself survives in the float window for dock-back.
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.iter_all_tabs().count(), 0);
    }

    #[test]
    fn dock_back_float_prefers_home_then_active_leaf() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        let home = workspace.tabs[0].id.clone();
        // Home group still exists: the pane stacks there, no new tab.
        workspace.dock_back(editor.clone(), &home);
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        // Home group gone: the pane lands in the active leaf instead.
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        workspace.dock_back(editor.clone(), "missing");
        assert_eq!(workspace.tabs.len(), 1);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        // Already docked: focuses instead of duplicating.
        let before = workspace.tabs[0].layout.iter_all_tabs().count();
        workspace.dock_back(editor.clone(), "missing");
        assert_eq!(workspace.tabs[0].layout.iter_all_tabs().count(), before);
    }

    #[test]
    fn dock_back_file_window_returns_to_source_group() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        workspace
            .move_pane_to_new_group_at(&editor, workspace.tabs.len())
            .expect("detach creates a tab");
        let home = workspace.tabs[0].id.clone();
        // Both tabs hold a single pane, so dock-back exchanges them: the
        // file window returns to its tab and selects it.
        assert!(workspace.move_pane_to_group(&editor, &home));
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        assert!(workspace.tabs[1].layout.find_tab(&shell).is_some());
        assert!(!workspace.move_pane_to_group(&editor, "missing"));
    }
}
