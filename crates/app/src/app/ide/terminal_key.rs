//! Alt+F12, the `JetBrains` Terminal tool window key.

use super::tool_windows::release_focus;
use crate::*;

impl App {
    /// IDE mode: show the terminal strip and focus a shell in it, or hide
    /// the strip when a strip shell has focus. Regular mode: focus the
    /// last shell, or open one. A shell that has focus stays focused.
    pub(crate) fn toggle_terminal_window(&mut self, ctx: &egui::Context) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let typing = shortcuts::terminal_has_focus(ctx);
        if self.preferences.ide_mode {
            if typing && self.strip_focused() {
                self.preferences.ide_terminal_collapsed = true;
                self.resync_active_from_dock();
                self.return_focus_to_work(ctx);
                return;
            }
            self.preferences.ide_terminal_collapsed = false;
            match self.strip_shell(&project) {
                Some(sid) => self.go_session(&sid),
                None => self.create_strip(),
            }
        } else {
            let active = self.active_session.clone().unwrap_or_default();
            if typing && self.live_shell(&project, &active) {
                return;
            }
            match self.last_shell(&project) {
                Some(sid) => self.go_session(&sid),
                None => self.create(None),
            }
        }
        // The shell takes the keyboard when it paints.
        release_focus(ctx);
    }

    fn live_shell(&self, project: &str, sid: &str) -> bool {
        self.state.sessions.iter().any(|session| {
            session.id == sid
                && session.project_id == project
                && session.kind == SessionKind::Shell
                && session.lifecycle.live()
        })
    }

    /// The strip's active shell when it is live, else its first live shell.
    fn strip_shell(&self, project: &str) -> Option<String> {
        let dock = self.preferences.ide_strip_docks.0.get(project)?;
        let surface = dock.main_surface();
        let active = surface
            .focused_leaf()
            .and_then(|node| surface.leaf(node).ok())
            .and_then(|leaf| leaf.tabs.get(leaf.active.0));
        match active {
            Some(Tab::Terminal(sid)) if self.live_shell(project, sid) => Some(sid.clone()),
            _ => self.strip_first_live(project),
        }
    }

    /// The project's last focused shell, else a live shell in its tabs.
    fn last_shell(&self, project: &str) -> Option<String> {
        let last = self
            .terminal_context
            .get(project)
            .filter(|sid| self.live_shell(project, sid));
        last.cloned().or_else(|| {
            let dock = self.layouts.get(project)?;
            dock.tabs
                .iter()
                .flat_map(|group| group.layout.iter_all_tabs())
                .find_map(|(_, tab)| match tab {
                    Tab::Terminal(sid) if self.live_shell(project, sid) => Some(sid.clone()),
                    _ => None,
                })
        })
    }
}
