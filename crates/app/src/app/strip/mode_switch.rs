//! IDE mode switches: shells move between the main dock and the strip as a
//! whole, keeping shell-only splits intact.

use std::collections::HashSet;

use crate::workspace::WorkspaceTab;
use crate::*;

/// What moving shells out of one main-dock workspace tab leaves behind.
enum SortedTab {
    /// Still holds non-shell panes.
    Kept(WorkspaceTab),
    /// Only shells: the whole layout moves so its splits survive.
    Carried(egui_dock::DockState<Tab>),
    /// Every pane left with the shells.
    Emptied,
}

struct StripHelpers;

impl StripHelpers {
    fn is_shell(pane: &Tab, shell_ids: &HashSet<String>) -> bool {
        matches!(pane, Tab::Terminal(sid) if shell_ids.contains(sid))
    }

    /// Takes the shells out of every workspace tab: shell-only layouts
    /// whole (`carried`), the rest pane by pane (`loose`).
    fn take_main_shells(
        workspace: &mut Workspace,
        shell_ids: &HashSet<String>,
    ) -> (Vec<egui_dock::DockState<Tab>>, Vec<Tab>) {
        let mut carried = Vec::new();
        let mut loose = Vec::new();
        let mut kept = Vec::new();
        for tab in std::mem::take(&mut workspace.tabs) {
            match Self::sort_tab(tab, shell_ids, &mut loose) {
                SortedTab::Kept(tab) => kept.push(tab),
                SortedTab::Carried(layout) => carried.push(layout),
                SortedTab::Emptied => {}
            }
        }
        workspace.tabs = kept;
        workspace.drop_empty_tabs();
        (carried, loose)
    }

    fn sort_tab(
        mut tab: WorkspaceTab,
        shell_ids: &HashSet<String>,
        loose: &mut Vec<Tab>,
    ) -> SortedTab {
        let panes: Vec<Tab> = tab
            .layout
            .iter_all_tabs()
            .map(|(_, pane)| pane.clone())
            .collect();
        let shells: Vec<Tab> = panes
            .iter()
            .filter(|pane| Self::is_shell(pane, shell_ids))
            .cloned()
            .collect();
        if shells.is_empty() {
            return SortedTab::Kept(tab);
        }
        if shells.len() == panes.len() {
            return SortedTab::Carried(tab.layout);
        }
        for pane in &shells {
            while let Some(path) = tab.layout.find_tab(pane) {
                tab.layout.remove_tab(path);
            }
        }
        loose.extend(shells);
        if tab.layout.iter_all_tabs().next().is_none() {
            return SortedTab::Emptied;
        }
        if tab
            .primary
            .as_ref()
            .is_some_and(|pane| Self::is_shell(pane, shell_ids))
        {
            tab.primary = tab
                .layout
                .iter_all_tabs()
                .next()
                .map(|(_, pane)| pane.clone());
        }
        SortedTab::Kept(tab)
    }

    /// An empty strip adopts the first carried layout; the other shells
    /// join its focused leaf, and its focused pane stays active.
    fn merge_into_first(
        mut carried: Vec<egui_dock::DockState<Tab>>,
        loose: &[Tab],
    ) -> egui_dock::DockState<Tab> {
        let mut dock = carried.remove(0);
        let focused = dock
            .main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| tab.clone());
        Self::push_missing(&mut dock, &carried, loose);
        if let Some(focused) = focused
            && let Some(path) = dock.find_tab(&focused)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
        dock
    }

    /// Adds every carried and loose pane `dock` lacks to its focused leaf.
    fn push_missing(
        dock: &mut egui_dock::DockState<Tab>,
        carried: &[egui_dock::DockState<Tab>],
        loose: &[Tab],
    ) {
        let carried_panes = carried
            .iter()
            .flat_map(|extra| extra.iter_all_tabs().map(|(_, pane)| pane));
        for pane in carried_panes.chain(loose) {
            if dock.find_tab(pane).is_none() {
                dock.push_to_focused_leaf(pane.clone());
            }
        }
    }
}

impl App {
    /// Mode is layout. Shells in the bottom strip move into the main dock
    /// when IDE mode turns off, preserving a shell-only split as one tab.
    /// Editors stay where they are. Returns whether anything moved.
    pub(crate) fn move_project_strip_to_main(&mut self, project: &str) -> bool {
        let shell_ids = self.shell_ids_for(project);
        let Some(layout) = self.take_strip_shell_dock(project, &shell_ids) else {
            return false;
        };
        let focus_moved = self.selected.as_deref() == Some(project)
            && self
                .active_session
                .as_ref()
                .is_some_and(|sid| shell_ids.contains(sid));
        let moved_ids: HashSet<String> = layout
            .iter_all_tabs()
            .filter_map(|(_, tab)| match tab {
                Tab::Terminal(sid) if shell_ids.contains(sid) => Some(sid.clone()),
                _ => None,
            })
            .collect();
        let show = {
            let workspace = self
                .layouts
                .entry(project.to_owned())
                .or_insert_with(Workspace::empty);
            // Drop a copy that was already in the main dock so the shell
            // renders once. Shells that were only in the main dock stay.
            workspace.remove_terminal_ids(&moved_ids);
            let blank = workspace
                .tabs
                .iter()
                .all(|tab| tab.layout.iter_all_tabs().next().is_none());
            let show = focus_moved || blank;
            workspace.adopt_dock(layout, show);
            show
        };
        let selected = self.selected.clone();
        if show && selected.as_deref() == Some(project) {
            let focused = self.layouts.get_mut(project).and_then(|workspace| {
                workspace
                    .main_surface_mut()
                    .find_active_focused()
                    .map(|(_, tab)| tab.clone())
            });
            if let Some(Tab::Terminal(sid)) = focused
                && shell_ids.contains(&sid)
            {
                self.active_session = Some(sid);
                self.non_terminal_selected = false;
            }
        }
        true
    }

    pub(crate) fn move_shells_to_main(&mut self) {
        let projects: Vec<String> = self.preferences.ide_strip_docks.0.keys().cloned().collect();
        for project in projects {
            self.move_project_strip_to_main(&project);
        }
    }

    /// Pull shell tabs out of the strip. A strip that is only shells moves
    /// as a whole dock so splits survive. Anything else stays in the strip.
    pub(crate) fn take_strip_shell_dock(
        &mut self,
        project: &str,
        shell_ids: &HashSet<String>,
    ) -> Option<egui_dock::DockState<Tab>> {
        let focus = self.active_session.clone();
        let strip = self.preferences.ide_strip_docks.0.get_mut(project)?;
        let shells: Vec<Tab> = strip
            .iter_all_tabs()
            .filter(|(_, tab)| StripHelpers::is_shell(tab, shell_ids))
            .map(|(_, tab)| tab.clone())
            .collect();
        if shells.is_empty() {
            return None;
        }
        let only_shells = strip
            .iter_all_tabs()
            .all(|(_, tab)| StripHelpers::is_shell(tab, shell_ids));
        if only_shells {
            return Some(std::mem::replace(strip, egui_dock::DockState::new(vec![])));
        }
        for tab in &shells {
            while let Some(path) = strip.find_tab(tab) {
                strip.remove_tab(path);
            }
        }
        let mut dock = egui_dock::DockState::new(shells);
        if let Some(sid) = focus {
            let pane = Tab::Terminal(sid);
            if let Some(path) = dock.find_tab(&pane) {
                let _ = dock.set_active_tab(path);
                dock.set_focused_node_and_surface(path.node_path());
            }
        }
        Some(dock)
    }

    /// Shells in the main dock move into the strip when IDE mode turns on.
    /// The first shell-only workspace tab keeps its splits. Editors stay.
    pub(crate) fn move_project_main_shells_to_strip(&mut self, project: &str) {
        let shell_ids = self.shell_ids_for(project);
        if shell_ids.is_empty() {
            return;
        }
        let Some(workspace) = self.layouts.get_mut(project) else {
            return;
        };
        let (carried, loose) = StripHelpers::take_main_shells(workspace, &shell_ids);
        if carried.is_empty() && loose.is_empty() {
            return;
        }
        let strip = self
            .preferences
            .ide_strip_docks
            .0
            .entry(project.to_owned())
            .or_insert_with(|| egui_dock::DockState::new(vec![]));
        let strip_empty = strip.iter_all_tabs().next().is_none();
        if strip_empty && !carried.is_empty() {
            *strip = StripHelpers::merge_into_first(carried, &loose);
            return;
        }
        StripHelpers::push_missing(strip, &carried, &loose);
    }

    pub(crate) fn move_shells_to_strip(&mut self) {
        let projects: Vec<String> = self.layouts.keys().cloned().collect();
        for project in projects {
            self.move_project_main_shells_to_strip(&project);
        }
    }
}
