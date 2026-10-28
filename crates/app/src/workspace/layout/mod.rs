//! Workspace layout edits: loading, adding, moving, and pruning tabs.
use super::model::{Workspace, WorkspaceTab};
use super::query::{contains_browser, validate_layout};
use crate::Tab;
use anyhow::{Context, Result, ensure};
use egui_dock::DockState;
use std::collections::{HashMap, HashSet};

mod cleanup;
mod moves;

pub struct AddAtFromInput<'a> {
    pub index: usize,
    pub id: String,
    pub pane: Tab,
    pub anchors: &'a [Tab],
}

pub struct MovePaneToSplitInput<'a> {
    pub pane: &'a Tab,
    pub dest_group: &'a str,
    pub dest: egui_dock::NodePath,
    pub split: egui_dock::Split,
}

pub struct ExchangeSinglePaneInput<'a> {
    pub src: usize,
    pub dst: usize,
    pub pane: &'a Tab,
    pub dest: egui_dock::NodePath,
    pub src_path: egui_dock::TabPath,
    pub dest_group: &'a str,
    pub other: Option<Tab>,
    pub src_len: usize,
    pub dst_len: usize,
}

impl Workspace {
    pub fn from_layout(layout: DockState<Tab>) -> Self {
        let id = terminator_core::id();
        let primary = layout.iter_all_tabs().next().map(|(_, tab)| tab.clone());
        Self {
            version: 2,
            active: id.clone(),
            return_tabs: HashMap::default(),
            recent_tabs: vec![id.clone()],
            tabs: vec![WorkspaceTab {
                id,
                name: None,
                pinned: false,
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
                matches!(workspace.version, 2..=9),
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
                    if let Tab::Browser { id, .. } | Tab::Vnc { id, .. } = pane
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
                if let Some(Tab::Vnc { id, .. }) = &mut group.primary
                    && id.is_empty()
                {
                    *id = terminator_core::id();
                }
            }
            for tab in &workspace.tabs {
                validate_layout(&tab.layout)?;
            }
            workspace
                .recent_tabs
                .retain(|id| workspace.tabs.iter().any(|tab| &tab.id == id));
            workspace.record_active();
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
            self.record_active();
            true
        } else {
            false
        }
    }

    pub fn add(&mut self, id: String, pane: Tab) {
        self.add_at(self.tabs.len(), id, pane);
    }

    pub fn add_from(&mut self, id: String, pane: Tab, anchors: &[Tab]) {
        self.add_at_from(crate::workspace::AddAtFromInput {
            index: self.tabs.len(),
            id,
            pane,
            anchors,
        });
    }

    pub fn add_at_from(&mut self, input: AddAtFromInput<'_>) {
        let AddAtFromInput {
            index,
            id,
            pane,
            anchors,
        } = input;

        let origin = anchors.iter().find_map(|anchor| {
            self.tabs
                .iter()
                .find(|tab| tab.layout.find_tab(anchor).is_some())
                .map(|tab| tab.id.clone())
        });
        self.add_at(index, id.clone(), pane);
        if !anchors.is_empty() {
            self.return_tabs.remove(&id);
            if let Some(origin) = origin {
                self.return_tabs.insert(id, origin);
            }
        }
    }

    pub fn add_at(&mut self, index: usize, id: String, mut pane: Tab) {
        if self.active != id
            && self
                .tabs
                .iter()
                .any(|tab| tab.id == self.active && tab.layout.iter_all_tabs().next().is_some())
        {
            self.return_tabs.insert(id.clone(), self.active.clone());
        }
        if let Tab::Browser { id, .. } | Tab::Vnc { id, .. } = &mut pane
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
                name: None,
                pinned: false,
                primary: Some(pane.clone()),
                layout: DockState::new(vec![pane]),
            },
        );
        self.active = id;
        self.record_active();
        self.main_surface_mut()
            .set_focused_node(egui_dock::NodeIndex::root());
    }

    pub fn close(&mut self, id: &str) {
        let previous = self.active_index();
        self.tabs.retain(|tab| tab.id != id);
        self.recent_tabs.retain(|recent| recent != id);
        self.normalize(previous);
    }

    /// Keep a bounded MRU list of top-level tabs. It is safe to call every
    /// frame; an unchanged active tab does not alter persisted state.
    pub fn record_active(&mut self) {
        if !self.tabs.iter().any(|tab| tab.id == self.active) {
            return;
        }
        if self.recent_tabs.first() == Some(&self.active) {
            return;
        }
        self.recent_tabs.retain(|id| id != &self.active);
        self.recent_tabs.insert(0, self.active.clone());
        self.recent_tabs.truncate(30);
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
}
