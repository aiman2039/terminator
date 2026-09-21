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

    pub fn strip_player(&mut self) {
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
fn contains_browser(workspace: &Workspace) -> bool {
    workspace.tabs.iter().any(|tab| {
        matches!(tab.primary, Some(Tab::Browser { .. }))
            || tab
                .layout
                .iter_all_tabs()
                .any(|(_, pane)| matches!(pane, Tab::Browser { .. }))
    })
}

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
}
