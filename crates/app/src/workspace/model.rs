//! Project-level tabs; each owns an independent split tree and pane focus.
use crate::Tab;
use egui_dock::DockState;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct WorkspaceTab {
    pub id: String,
    /// Sidebar-given group name. `None` renders as an abstract `Tab N`;
    /// the group's identity never borrows a member terminal's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Pinned top-level tabs stay open during bulk-close actions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    pub primary: Option<Tab>,
    pub layout: DockState<Tab>,
}

/// What the sidebar calls a workspace tab: its custom name, or the
/// abstract position label `Tab N` (1-based, layout order).
pub(crate) fn group_display_name(name: Option<&str>, index: usize) -> String {
    match name {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => format!("Tab {}", index.saturating_add(1)),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Workspace {
    pub version: u32,
    pub active: String,
    pub tabs: Vec<WorkspaceTab>,
    /// Tab identities survive reordering and a GUI restart.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub return_tabs: std::collections::HashMap<String, String>,
    /// Most recently selected top-level tabs, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_tabs: Vec<String>,
}

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
                name: None,
                pinned: false,
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
