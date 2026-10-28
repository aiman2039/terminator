//! After-paint sync: prune what was not painted and point the metadata,
//! refresh, and focus requests at what is now visible.

use crate::*;

impl App {
    pub(super) fn save_changed_preferences(&mut self) {
        if self.preferences_writable
            && !self.preferences_pending
            && self.preferences != self.preferences_saved
        {
            let _ = self.jobs.send(Job::Preferences(self.preferences.clone()));
            self.preferences_pending = true;
        }
    }

    /// Drops images, VNC views, markdown, and backends no pane painted.
    pub(super) fn prune_unpainted(&mut self, ctx: &egui::Context) {
        // Floating panes render after these prunes (`paint_floating` runs
        // last), so a frame that only marked docked panes would drop every
        // floated backend/image/preview and re-attach it on the next paint:
        // an attach storm that stutters the whole window. Count floats as
        // visible up front; the float paint re-marks them anyway.
        self.seed_floating_visibility();
        self.images
            .retain(|path, _| self.visible_images.contains(path));
        // View state and remote sessions both follow the tabs now that the
        // dock is checked back in and every copy has had its say.
        self.prune_vnc_views();
        self.drain_pending_vnc_disconnect();
        self.markdown.end_frame(ctx);
        self.backends
            .retain(|sid, _| self.visible_sessions.contains(sid));
    }

    /// Remembers the active shell per project and tells the daemon when
    /// focus moved.
    pub(super) fn sync_terminal_focus(&mut self) {
        if let Some(session) =
            self.state.sessions.iter().find(|s| {
                Some(&s.id) == self.active_session.as_ref() && s.kind == SessionKind::Shell
            })
        {
            self.terminal_context
                .insert(session.project_id.clone(), session.id.clone());
        }
        if self.active_session != self.last_focus {
            if let Some(session) = &self.active_session {
                self.send(Request::Focus {
                    session: session.clone(),
                });
            }
            self.last_focus = self.active_session.clone();
        }
    }

    pub(super) fn sync_metadata_request(&mut self) {
        let next_metadata = self.cwd().map(|cwd| metadata_refresh::Request {
            cwd,
            identity: self
                .context_session()
                .and_then(|s| s.pid.map(|pid| (pid, s.created))),
            include_pr: self.state.settings.pr_metadata,
            generation: self.metadata_generation,
        });
        if next_metadata == self.metadata_request {
            return;
        }
        self.metadata_generation = self.metadata_generation.wrapping_add(1);
        self.metadata = None;
        self.metadata_request = next_metadata.map(|mut r| {
            r.generation = self.metadata_generation;
            r
        });
        let _ = self.metadata_jobs.send(self.metadata_request.clone());
    }

    pub(super) fn sync_refresh_request(&mut self) {
        self.visible_dirs.sort();
        self.visible_dirs.dedup();
        let right_files = self.preferences.visible
            && matches!(
                self.preferences.tool,
                SidebarTool::Explorer | SidebarTool::Git
            );
        let left_files = self.preferences.left_visible
            && matches!(
                self.preferences.left_tool,
                Some(SidebarTool::Explorer | SidebarTool::Git)
            );
        let wants_files = right_files || left_files;
        let include_stats = (self.preferences.visible && self.preferences.tool == SidebarTool::Git)
            || (self.preferences.left_visible
                && self.preferences.left_tool == Some(SidebarTool::Git));
        let next = self
            .cwd()
            .filter(|_| wants_files)
            .map(|cwd| refresh::Request {
                cwd,
                generation: self.refresh_generation,
                directories: self.visible_dirs.clone(),
                include_stats,
            });
        if next != self.refresh_request {
            self.send_refresh_request(next);
        }
    }

    fn send_refresh_request(&mut self, next: Option<refresh::Request>) {
        self.refresh_generation = self.refresh_generation.saturating_add(1);
        let next = next.map(|mut r| {
            r.generation = self.refresh_generation;
            r
        });
        let cwd = next.as_ref().map(|r| r.cwd.clone());
        if self.context_path != cwd {
            self.context = None;
            self.dirs.clear();
            self.directory_errors.clear();
        }
        self.context_path = cwd;
        self.refresh_request.clone_from(&next);
        let _ = self.refresh.send(next);
    }

    pub(super) fn dismiss_stale_hover_popup(&mut self, ctx: &egui::Context) {
        if self.hover_popup.as_ref().is_some_and(|popup| {
            !self.visible_sessions.contains(&popup.session)
                || !self.backends.contains_key(&popup.session)
        }) {
            self.dismiss_hover_popup(ctx);
        }
    }
}
