use super::super::appearance;
use eframe::egui::{self};
use std::collections::HashSet;
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn shell_ids_for(&self, project: &str) -> HashSet<String> {
        self.state
            .sessions
            .iter()
            .filter(|session| session.project_id == project && session.kind == SessionKind::Shell)
            .map(|session| session.id.clone())
            .collect()
    }

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
            .filter(|(_, tab)| matches!(tab, Tab::Terminal(sid) if shell_ids.contains(sid)))
            .map(|(_, tab)| tab.clone())
            .collect();
        if shells.is_empty() {
            return None;
        }
        let only_shells = strip
            .iter_all_tabs()
            .all(|(_, tab)| matches!(tab, Tab::Terminal(sid) if shell_ids.contains(sid)));
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
        let mut carried: Vec<egui_dock::DockState<Tab>> = Vec::new();
        let mut loose: Vec<Tab> = Vec::new();
        {
            let Some(workspace) = self.layouts.get_mut(project) else {
                return;
            };
            let tabs = std::mem::take(&mut workspace.tabs);
            let mut kept = Vec::new();
            for tab in tabs {
                let panes: Vec<Tab> = tab
                    .layout
                    .iter_all_tabs()
                    .map(|(_, pane)| pane.clone())
                    .collect();
                let shells: Vec<Tab> = panes
                    .iter()
                    .filter(|pane| matches!(pane, Tab::Terminal(sid) if shell_ids.contains(sid)))
                    .cloned()
                    .collect();
                if shells.is_empty() {
                    kept.push(tab);
                    continue;
                }
                if shells.len() == panes.len() {
                    carried.push(tab.layout);
                    continue;
                }
                let mut tab = tab;
                for pane in &shells {
                    while let Some(path) = tab.layout.find_tab(pane) {
                        tab.layout.remove_tab(path);
                    }
                }
                loose.extend(shells);
                if tab.layout.iter_all_tabs().next().is_none() {
                    continue;
                }
                if tab.primary.as_ref().is_some_and(
                    |pane| matches!(pane, Tab::Terminal(sid) if shell_ids.contains(sid)),
                ) {
                    tab.primary = tab
                        .layout
                        .iter_all_tabs()
                        .next()
                        .map(|(_, pane)| pane.clone());
                }
                kept.push(tab);
            }
            workspace.tabs = kept;
            workspace.drop_empty_tabs();
        }
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
            let mut dock = carried.remove(0);
            let focused = dock
                .main_surface_mut()
                .find_active_focused()
                .map(|(_, tab)| tab.clone());
            for extra in &carried {
                for (_, pane) in extra.iter_all_tabs() {
                    if dock.find_tab(pane).is_none() {
                        dock.push_to_focused_leaf(pane.clone());
                    }
                }
            }
            for pane in &loose {
                if dock.find_tab(pane).is_none() {
                    dock.push_to_focused_leaf(pane.clone());
                }
            }
            if let Some(focused) = focused
                && let Some(path) = dock.find_tab(&focused)
            {
                let _ = dock.set_active_tab(path);
                dock.set_focused_node_and_surface(path.node_path());
            }
            *strip = dock;
            return;
        }
        for dock in &carried {
            for (_, pane) in dock.iter_all_tabs() {
                if strip.find_tab(pane).is_none() {
                    strip.push_to_focused_leaf(pane.clone());
                }
            }
        }
        for pane in &loose {
            if strip.find_tab(pane).is_none() {
                strip.push_to_focused_leaf(pane.clone());
            }
        }
    }

    pub(crate) fn move_shells_to_strip(&mut self) {
        let projects: Vec<String> = self.layouts.keys().cloned().collect();
        for project in projects {
            self.move_project_main_shells_to_strip(&project);
        }
    }

    /// Match the baselines `sync_active_session` compares, so a mode switch
    /// does not look like the user clicked a dock.
    pub(crate) fn note_focus_baselines(&mut self) {
        let project = self.selected.clone();
        self.last_main_focus = project.as_ref().and_then(|project| {
            self.layouts.get_mut(project).and_then(|dock| {
                dock.main_surface_mut()
                    .find_active_focused()
                    .map(|(_, tab)| tab.clone())
            })
        });
        self.last_strip_focus = project.as_ref().and_then(|project| {
            self.preferences
                .ide_strip_docks
                .0
                .get_mut(project)
                .and_then(|strip| {
                    strip
                        .main_surface_mut()
                        .find_active_focused()
                        .map(|(_, tab)| tab.clone())
                })
        });
    }

    /// Strip-dock membership: a session lives in exactly one dock, so this
    /// doubles as the "not in the main dock" check.
    pub(crate) fn is_strip_session(&self, project: &str, sid: &str) -> bool {
        let pane = Tab::Terminal(sid.into());
        self.preferences
            .ide_strip_docks
            .0
            .get(project)
            .is_some_and(|dock| dock.find_tab(&pane).is_some())
    }

    /// Drop `sid` from every strip dock (close, terminate, session end).
    pub(crate) fn drop_strip_session(&mut self, sid: &str) {
        let pane = Tab::Terminal(sid.into());
        for dock in self.preferences.ide_strip_docks.0.values_mut() {
            while let Some(path) = dock.find_tab(&pane) {
                dock.remove_tab(path);
            }
        }
    }

    /// Remember a strip shell to move once its dock is not checked out.
    pub(crate) fn queue_strip_move(&mut self, sid: &str) {
        self.move_strip_to_main = Some(sid.to_owned());
    }

    /// Apply a queued strip move. No-op while the selected project's strip
    /// dock is checked out for paint; the caller retries after inserting it.
    pub(crate) fn drain_strip_move(&mut self) {
        let Some(sid) = self.move_strip_to_main.clone() else {
            return;
        };
        let checked_out = self.selected.as_ref().is_some_and(|project| {
            !self.preferences.ide_strip_docks.0.contains_key(project)
                && self
                    .state
                    .sessions
                    .iter()
                    .any(|session| session.id == sid && session.project_id == *project)
        });
        if checked_out {
            return;
        }
        self.move_strip_to_main = None;
        self.move_strip_session_to_main(&sid);
    }

    /// Move one strip shell into its own main-pane workspace tab. IDE mode
    /// stays on. Split siblings stay in the strip. Editors stay where they
    /// are. Returns false when `sid` is not a strip shell.
    pub(crate) fn move_strip_session_to_main(&mut self, sid: &str) -> bool {
        let Some(project) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == sid && session.kind == SessionKind::Shell)
            .map(|session| session.project_id.clone())
        else {
            return false;
        };
        if !self.is_strip_session(&project, sid) {
            return false;
        }
        let pane = Tab::Terminal(sid.to_owned());
        self.drop_strip_session(sid);
        {
            let workspace = self
                .layouts
                .entry(project.clone())
                .or_insert_with(Workspace::empty);
            workspace.remove_terminal_ids(&HashSet::from([sid.to_owned()]));
            if workspace.contains(&pane) {
                workspace.activate_containing(&pane);
            } else {
                workspace.add(id(), pane.clone());
            }
        }
        self.hide_center_overlay();
        if self.selected.as_deref() != Some(project.as_str()) {
            self.select_project(project);
        }
        self.active_session = Some(sid.to_owned());
        self.non_terminal_selected = false;
        self.focus_tab = Some(pane);
        self.note_focus_baselines();
        self.save_layouts();
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
        true
    }

    /// Remember a main-pane shell to move once its workspace is checked in.
    pub(crate) fn queue_main_to_strip(&mut self, sid: &str) {
        self.move_main_to_strip = Some(sid.to_owned());
    }

    pub(crate) fn drain_main_to_strip(&mut self) {
        let Some(sid) = self.move_main_to_strip.take() else {
            return;
        };
        self.move_main_session_to_strip(&sid);
    }

    /// Move one main-pane shell into the IDE strip. IDE mode stays on and the
    /// strip is shown. Other main-pane tabs, including editors, stay. Returns
    /// false when `sid` is not a main-pane shell or IDE mode is off.
    pub(crate) fn move_main_session_to_strip(&mut self, sid: &str) -> bool {
        if !self.preferences.ide_mode {
            return false;
        }
        let Some(project) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == sid && session.kind == SessionKind::Shell)
            .map(|session| session.project_id.clone())
        else {
            return false;
        };
        if self.is_strip_session(&project, sid) {
            return false;
        }
        let pane = Tab::Terminal(sid.to_owned());
        let Some(workspace) = self.layouts.get_mut(&project) else {
            return false;
        };
        if !workspace.contains(&pane) {
            return false;
        }
        workspace.remove_terminal_ids(&HashSet::from([sid.to_owned()]));
        self.insert_strip(&project, pane.clone(), None);
        self.preferences.ide_terminal_collapsed = false;
        self.hide_center_overlay();
        if self.selected.as_deref() != Some(project.as_str()) {
            self.select_project(project);
        }
        self.active_session = Some(sid.to_owned());
        self.non_terminal_selected = false;
        self.focus_strip_tab = Some(pane);
        self.note_focus_baselines();
        self.save_layouts();
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
        true
    }

    /// Take a strip shell into `dock` as its own workspace tab. The caller
    /// then lands it on the drop target. The workspace is checked out, so
    /// the layout save waits until it is inserted again.
    pub(crate) fn pull_strip_shell_into(&mut self, dock: &mut Workspace, sid: &str) -> bool {
        let Some(project) = self.selected.clone() else {
            return false;
        };
        let shell = self.state.sessions.iter().any(|session| {
            session.id == sid && session.kind == SessionKind::Shell && session.project_id == project
        });
        if !shell || !self.is_strip_session(&project, sid) {
            return false;
        }
        let pane = Tab::Terminal(sid.to_owned());
        self.drop_strip_session(sid);
        if dock.contains(&pane) {
            dock.activate_containing(&pane);
        } else {
            dock.add(id(), pane.clone());
        }
        self.active_session = Some(sid.to_owned());
        self.non_terminal_selected = false;
        self.focus_tab = Some(pane);
        self.hide_center_overlay();
        self.pending_layout_save = true;
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
        true
    }

    /// Drop a strip shell onto a main-pane leaf. Single panes exchange places.
    /// A failed landing still leaves the shell in its own workspace tab.
    pub(crate) fn land_strip_shell_on_leaf(
        &mut self,
        dock: &mut Workspace,
        sid: &str,
        group: &str,
        path: egui_dock::NodePath,
        zone: PaneDropZone,
    ) -> bool {
        if !self.pull_strip_shell_into(dock, sid) {
            return false;
        }
        let pane = Tab::Terminal(sid.to_owned());
        let placed = match zone.split() {
            None => dock.move_pane_to_group_leaf(&pane, group, path),
            Some(split) => dock.move_pane_to_split(&pane, group, path, split),
        };
        placed || dock.contains(&pane)
    }

    /// A strip-tab drag that has left the strip becomes a pane drag so the
    /// main pane can accept the drop. Releasing inside the strip does not.
    pub(crate) fn track_strip_tab_drag(&mut self, ui: &egui::Ui, strip_rect: egui::Rect) {
        let Some(sid) = self.strip_tab_drag.clone() else {
            return;
        };
        let down = ui.input(|input| input.pointer.any_down());
        let released = ui.input(|input| input.pointer.any_released());
        if !down && !released {
            self.strip_tab_drag = None;
            return;
        }
        let outside = ui
            .input(|input| input.pointer.interact_pos().or(input.pointer.latest_pos()))
            .is_some_and(|pos| !strip_rect.contains(pos));
        if outside && self.pane_drag.is_none() {
            self.pane_drag = Some(Tab::Terminal(sid));
            self.pane_drag_from_strip = true;
        }
        if released || !down {
            self.strip_tab_drag = None;
        }
    }

    /// Drop a main-pane shell onto the IDE strip. A strip-origin drag released
    /// back here cancels instead, leaving the dock reorder in place.
    pub(crate) fn take_main_drop_on_strip(&mut self, ui: &egui::Ui, strip_rect: egui::Rect) {
        let Some(Tab::Terminal(sid)) = self.pane_drag.clone() else {
            return;
        };
        let over = ui
            .input(|input| input.pointer.interact_pos().or(input.pointer.latest_pos()))
            .is_some_and(|pos| strip_rect.contains(pos));
        if !over {
            return;
        }
        let released = ui.input(|input| input.pointer.any_released());
        let down = ui.input(|input| input.pointer.any_down());
        if self.pane_drag_from_strip {
            if released {
                self.end_pane_drag();
            }
            return;
        }
        if down && !released {
            ui.painter().rect_stroke(
                strip_rect,
                2,
                egui::Stroke::new(2.0, appearance::color(&self.theme.accent)),
                egui::StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
            return;
        }
        if released {
            self.move_main_session_to_strip(&sid);
            self.end_pane_drag();
        }
    }

    /// Focus a strip tab: activate it in the strip dock, make it the active
    /// session, tell the daemon.
    pub(crate) fn activate_strip_session(&mut self, project: &str, sid: &str) {
        let pane = Tab::Terminal(sid.into());
        if let Some(dock) = self.preferences.ide_strip_docks.0.get_mut(project)
            && let Some(path) = dock.find_tab(&pane)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
        self.active_session = Some(sid.into());
        self.send(Request::Focus {
            session: sid.into(),
        });
    }

    /// First live shell tab in the strip dock, for focus healing.
    pub(crate) fn strip_first_live(&self, project: &str) -> Option<String> {
        let dock = self.preferences.ide_strip_docks.0.get(project)?;
        dock.iter_all_tabs().find_map(|(_, tab)| match tab {
            Tab::Terminal(sid)
                if self.state.sessions.iter().any(|s| {
                    &s.id == sid && s.kind == SessionKind::Shell && s.lifecycle.live()
                }) =>
            {
                Some(sid.clone())
            }
            _ => None,
        })
    }

    /// The bottom strip is on screen. Hidden docks stay in preferences and
    /// must not take keyboard focus.
    pub(crate) fn ide_strip_visible(&self) -> bool {
        self.preferences.ide_mode && !self.preferences.ide_terminal_collapsed
    }
}
