//! Project-level tabs; each owns an independent split tree and pane focus.
use crate::Tab;
use anyhow::{Context, Result, ensure};
use egui_dock::DockState;
use serde::{Deserialize, Serialize};
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
            let workspace: Self = serde_json::from_value(terminator_core::sanitize_layout(value))
                .context("Invalid project tabs")?;
            ensure!(
                matches!(workspace.version, 2..=5),
                "Unsupported project tab layout version"
            );
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
            .and_then(|node| self.main_surface()[node].get_leaf())
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
        self.tabs
            .iter()
            .any(|tab| tab.layout.find_tab(pane).is_some())
    }
    pub fn add(&mut self, id: String, pane: Tab) {
        match pane {
            Tab::Player => self.version = 5,
            Tab::Html { .. } => self.version = self.version.max(4),
            Tab::Image { .. } => self.version = self.version.max(3),
            _ => {}
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.tabs.push(WorkspaceTab {
            id: id.clone(),
            primary: Some(pane.clone()),
            layout: DockState::new(vec![pane]),
        });
        self.active = id;
        self.main_surface_mut()
            .set_focused_node(egui_dock::NodeIndex::root());
    }
    pub fn close(&mut self, id: &str) {
        let previous = self.active_index();
        self.tabs.retain(|tab| tab.id != id);
        self.normalize(previous);
    }
    pub fn remove_session(&mut self, sid: &str) {
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
            *self = Self::empty();
        } else if !self.tabs.iter().any(|tab| tab.id == self.active) {
            self.active = self.tabs[previous.saturating_sub(1).min(self.tabs.len() - 1)]
                .id
                .clone();
        }
    }
}
// Serde restores raw docking indices without the checks used by UI setters.
// Reject invalid persisted focus before any App or renderer indexing occurs.
fn validate_layout(layout: &DockState<Tab>) -> Result<()> {
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
                tree.iter().nth(focus.0).is_some_and(|node| node.is_leaf()),
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
        &self.tabs[self.active_index()].layout
    }
}
impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let index = self.active_index();
        &mut self.tabs[index].layout
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        for version in [2, 3, 4, 5] {
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
        workspace.add(
            "html".into(),
            Tab::Html {
                path: "/page.html".into(),
            },
        );
        assert_eq!(workspace.version, 4);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Html {
            path: "/page.html".into()
        }));
        workspace.add("player".into(), Tab::Player);
        assert_eq!(workspace.version, 5);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Player));
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
}
