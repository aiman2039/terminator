//! The IDE strip's dock membership and focus.

use std::collections::HashSet;

use crate::*;

mod mode_switch;
mod session_moves;

pub(crate) struct LandStripShellOnLeafInput<'a> {
    pub(crate) dock: &'a mut Workspace,
    pub(crate) sid: &'a str,
    pub(crate) group: &'a str,
    pub(crate) path: egui_dock::NodePath,
    pub(crate) zone: PaneDropZone,
}

impl App {
    pub(crate) fn shell_ids_for(&self, project: &str) -> HashSet<String> {
        self.state
            .sessions
            .iter()
            .filter(|session| session.project_id == project && session.kind == SessionKind::Shell)
            .map(|session| session.id.clone())
            .collect()
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
