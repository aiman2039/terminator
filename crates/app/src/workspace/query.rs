use super::model::Workspace;
use crate::Tab;
use anyhow::{Result, ensure};
use egui_dock::DockState;

impl Workspace {
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
}

// Serde restores raw docking indices without the checks used by UI setters.
// Reject invalid persisted focus before any App or renderer indexing occurs.
pub(super) fn contains_browser(workspace: &Workspace) -> bool {
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
