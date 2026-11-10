//! After-paint sync: prune what was not painted and point the metadata,
//! refresh, and focus requests at what is now visible.

use crate::*;

/// Hidden terminals that stay attached (see `prune_hidden_backends`).
const KEEP_HIDDEN_BACKENDS: usize = 4;

impl App {
    pub(crate) fn save_changed_preferences(&mut self) {
        let backing_off = self
            .preferences_retry_at
            .is_some_and(|at| Instant::now() < at && !self.exit.active());
        if self.preferences_writable
            && !self.preferences_pending
            && !backing_off
            && self.preferences != self.preferences_saved
        {
            // A refused save answers with its error; no save is pending.
            self.preferences_pending = self
                .jobs
                .send(Job::Preferences(self.preferences.clone()))
                .is_ok();
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
        self.tables.prune();
        // View state and remote sessions both follow the tabs now that the
        // dock is checked back in and every copy has had its say.
        self.prune_vnc_views();
        self.remote_apps_tick(ctx);
        self.markdown.end_frame(ctx);
        self.prune_hidden_backends();
    }

    /// Hidden terminals stay attached for a quick switch back: the
    /// [`KEEP_HIDDEN_BACKENDS`] most recently painted ones. Re-attaching
    /// spawns a helper process and replays the screen; more than a few
    /// would keep parsing output nobody sees.
    fn prune_hidden_backends(&mut self) {
        let hidden: Vec<(Option<Instant>, String)> = self
            .backends
            .iter()
            .filter(|(sid, _)| !self.visible_sessions.contains(*sid))
            .map(|(sid, backend)| (backend.last_painted(), sid.clone()))
            .collect();
        for sid in hidden_backends_to_drop(hidden, KEEP_HIDDEN_BACKENDS) {
            self.detach_backend(&sid);
        }
    }
}

/// The hidden backends past the `keep` most recently painted ones.
pub(crate) fn hidden_backends_to_drop(
    mut hidden: Vec<(Option<Instant>, String)>,
    keep: usize,
) -> Vec<String> {
    hidden.sort_by_key(|(painted, _)| std::cmp::Reverse(*painted));
    hidden.into_iter().skip(keep).map(|(_, sid)| sid).collect()
}

impl App {
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

    pub(crate) fn sync_refresh_request(&mut self) {
        self.visible_dirs.sort();
        self.visible_dirs.dedup();
        let right_files = self.preferences.visible
            && matches!(
                self.preferences.tool,
                SidebarTool::Explorer | SidebarTool::Git
            );
        let left_files = self.preferences.left_shows(SidebarTool::Explorer)
            || self.preferences.left_shows(SidebarTool::Git);
        let wants_files = right_files || left_files;
        let include_stats = (self.preferences.visible && self.preferences.tool == SidebarTool::Git)
            || self.preferences.left_shows(SidebarTool::Git);
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

#[cfg(test)]
mod hidden_backend_tests {
    use super::hidden_backends_to_drop;
    use std::time::{Duration, Instant};

    #[test]
    fn the_most_recently_painted_hidden_terminals_stay_attached() {
        let now = Instant::now();
        let ago = |secs| now.checked_sub(Duration::from_secs(secs));
        let hidden = vec![
            (ago(50), "old".to_owned()),
            (ago(1), "newest".to_owned()),
            (None, "never".to_owned()),
            (ago(10), "recent".to_owned()),
        ];
        assert_eq!(hidden_backends_to_drop(hidden.clone(), 2), ["old", "never"]);
        assert!(hidden_backends_to_drop(hidden, 4).is_empty());
    }

    /// A pruned hidden terminal leaves no attach bookkeeping, and the late
    /// exit of its helper changes nothing.
    #[cfg(unix)]
    #[test]
    fn pruned_hidden_terminal_drops_its_attach_bookkeeping() {
        use std::os::unix::process::ExitStatusExt;
        let (mut app, ctx, _dir) = crate::app::tests::support::fixture();
        let sessions: Vec<String> = (0..=super::KEEP_HIDDEN_BACKENDS)
            .map(|n| format!("hidden-{n}"))
            .collect();
        for (id, sid) in (100_u64..).zip(&sessions) {
            let backend = egui_term::TerminalBackend::new(
                id,
                ctx.clone(),
                app.pty_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec!["-c".into(), "sleep 30".into()],
                    ..Default::default()
                },
            )
            .unwrap();
            app.backends.insert(sid.clone(), backend);
            app.backend_ids.insert(id, sid.clone());
            app.attach_started.insert(sid.clone(), Instant::now());
            app.attach_error.insert(sid.clone(), "old failure".into());
        }
        app.prune_hidden_backends();
        let pruned: Vec<&String> = sessions
            .iter()
            .filter(|sid| !app.backends.contains_key(*sid))
            .collect();
        assert_eq!(pruned.len(), 1);
        let pruned = pruned[0].clone();
        let id = 100 + sessions.iter().position(|sid| *sid == pruned).unwrap() as u64;
        assert!(!app.backend_ids.contains_key(&id));
        assert!(!app.attach_started.contains_key(&pruned));
        assert!(!app.attach_error.contains_key(&pruned));
        app.pty_tx
            .send((
                id,
                egui_term::PtyEvent::ChildExit(ExitStatusExt::from_raw(256)),
            ))
            .unwrap();
        app.pty_tx.send((id, egui_term::PtyEvent::Exit)).unwrap();
        app.process_updates(&ctx);
        assert!(app.attach_failed.is_empty());
        assert!(!app.attach_budget.contains_key(&pruned));
        assert!(!app.attach_error.contains_key(&pruned));
        assert_eq!(app.backends.len(), super::KEEP_HIDDEN_BACKENDS);
    }
}
