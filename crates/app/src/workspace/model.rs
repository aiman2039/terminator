//! Project-level tabs; each owns an independent split tree and pane focus.
use crate::Tab;
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
