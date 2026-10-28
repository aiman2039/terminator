//! Moving single shells between the strip and the main dock: context
//! menu moves and cross-dock drags.

use std::collections::HashSet;

use super::LandStripShellOnLeafInput;
use crate::*;

impl App {
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
        input: LandStripShellOnLeafInput<'_>,
    ) -> bool {
        let LandStripShellOnLeafInput {
            dock,
            sid,
            group,
            path,
            zone,
        } = input;
        if !self.pull_strip_shell_into(dock, sid) {
            return false;
        }
        let pane = Tab::Terminal(sid.to_owned());
        let placed = match zone.split() {
            None => dock.move_pane_to_group_leaf(&pane, group, path),
            Some(split) => dock.move_pane_to_split(crate::workspace::MovePaneToSplitInput {
                pane: &pane,
                dest_group: group,
                dest: path,
                split,
            }),
        };
        placed || dock.contains(&pane)
    }

    /// A strip-tab drag that has left the strip becomes a pane drag so the
    /// main pane can accept the drop. Releasing inside the strip does not.
    pub(crate) fn track_strip_tab_drag(&mut self, ui: &egui::Ui, strip_rect: egui::Rect) {
        let Some(sid) = self.drag.strip_tab.clone() else {
            return;
        };
        let down = ui.input(|input| input.pointer.any_down());
        let released = ui.input(|input| input.pointer.any_released());
        if !down && !released {
            self.drag.strip_tab = None;
            return;
        }
        let outside = ui
            .input(|input| input.pointer.interact_pos().or(input.pointer.latest_pos()))
            .is_some_and(|pos| !strip_rect.contains(pos));
        if outside && self.drag.pane.is_none() {
            self.drag.pane = Some(Tab::Terminal(sid));
            self.drag.pane_from_strip = true;
        }
        if released || !down {
            self.drag.strip_tab = None;
        }
    }

    /// Drop a main-pane shell onto the IDE strip. A strip-origin drag released
    /// back here cancels instead, leaving the dock reorder in place.
    pub(crate) fn take_main_drop_on_strip(&mut self, ui: &egui::Ui, strip_rect: egui::Rect) {
        let Some(Tab::Terminal(sid)) = self.drag.pane.clone() else {
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
        if self.drag.pane_from_strip {
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
}
