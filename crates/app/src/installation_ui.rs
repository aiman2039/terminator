//! Recovery uses the ordinary session navigation and safe idle restart paths.
use super::*;

impl App {
    pub(super) fn installation_problem(&self) -> bool {
        self.installation_error.is_some() || self.state.attachment_helper_available == Some(false)
    }

    pub(super) fn open_installation_settings(&mut self) {
        self.open_settings();
        self.settings_section = SettingsSection::Updates;
    }

    pub(super) fn maybe_upgrade_idle_daemon(&mut self) {
        if self.exit.active()
            || !self.connected
            || self.repair_pending
            || !can_retire_daemon(&self.state)
            || self.automatic_repair_attempt.as_ref() == Some(&self.state.generation)
        {
            return;
        }
        // One automatic attempt per generation; failures remain manually retryable.
        self.automatic_repair_attempt = Some(self.state.generation.clone());
        self.begin_installation_repair();
    }

    pub(super) fn restart_session_button(&mut self, ui: &mut egui::Ui, small: bool) {
        if !self.connected || !can_restart_service(&self.state) {
            return;
        }
        let label = if self.restart_pending {
            "Restarting…"
        } else {
            "Stop all sessions and restart"
        };
        let enabled = !self.restart_pending && !self.repair_pending && !self.exit.active();
        let response = if small {
            ui.add_enabled(enabled, egui::Button::new(label).small())
        } else {
            ui.add_enabled(enabled, egui::Button::new(label))
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "restart-session-service", response.rect);
        if response.clicked() {
            self.restart_confirm = true;
        }
    }

    pub(super) fn begin_session_restart(&mut self) {
        if self.restart_pending
            || self.repair_pending
            || self.exit.active()
            || !can_restart_service(&self.state)
        {
            return;
        }
        self.restart_confirm = false;
        match self.jobs.send(Job::RestartSessionService(
            recovery::RestartInventory::capture(&self.state),
        )) {
            Ok(()) => self.restart_pending = true,
            Err(_) => {
                self.error = Some("Restart worker disconnected. Reopen Terminator to retry.".into())
            }
        }
    }

    pub(super) fn begin_installation_repair(&mut self) {
        if self.repair_pending
            || !self.connected
            || (!can_retire_daemon(&self.state) && self.state.generations.is_empty())
        {
            return;
        }
        self.finish_rename(true);
        match self.exit_checkpoint() {
            Ok(checkpoint) => {
                match self.jobs.send(Job::RepairInstallation(
                    self.state.generation.clone(),
                    checkpoint,
                )) {
                    Ok(()) => self.repair_pending = true,
                    Err(_) => {
                        self.error = Some(
                            "Installation worker disconnected. Reopen Terminator to retry.".into(),
                        )
                    }
                }
            }
            Err(error) => {
                self.error = Some(format!("Could not save workspace before repair: {error:#}"))
            }
        }
    }

    /// The GUI has loaded daemon state before but the snapshot poller can no
    /// longer reach the session service (reboot, crash, or a wiped runtime
    /// directory). Session records and window-close persistence are stale
    /// until the service is started again.
    pub(super) fn service_disconnected(&self) -> bool {
        self.state_loaded && !self.connected
    }

    /// Terminal tabs without a matching session record in the latest daemon
    /// state. Only reported while connected so a stale snapshot never prunes
    /// tabs whose sessions may still exist.
    pub(super) fn unavailable_tabs(&self) -> Vec<String> {
        if !self.connected {
            return Vec::new();
        }
        let mut missing: Vec<String> = self
            .layouts
            .values()
            .flat_map(|workspace| workspace.iter_all_tabs())
            .chain(
                self.preferences
                    .ide_strip_docks
                    .0
                    .values()
                    .flat_map(|dock| dock.iter_all_tabs()),
            )
            .filter_map(|(_, tab)| match tab {
                Tab::Terminal(sid) => {
                    (!self.state.sessions.iter().any(|s| s.id == *sid)).then(|| sid.clone())
                }
                _ => None,
            })
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    pub(super) fn start_service_button(&mut self, ui: &mut egui::Ui, small: bool) {
        if !self.service_disconnected() {
            return;
        }
        let label = if self.service_start_pending {
            "Starting…"
        } else {
            "Start session service"
        };
        let enabled = !self.service_start_pending && !self.exit.active();
        let response = if small {
            ui.add_enabled(enabled, egui::Button::new(label).small())
        } else {
            ui.add_enabled(enabled, egui::Button::new(label))
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "start-session-service", response.rect);
        if response.clicked() {
            self.begin_service_start();
        }
    }

    pub(super) fn begin_service_start(&mut self) {
        if self.service_start_pending || self.connected {
            return;
        }
        match self.jobs.send(Job::StartSessionService) {
            Ok(()) => self.service_start_pending = true,
            Err(_) => {
                self.error = Some("Service worker disconnected. Reopen Terminator to retry.".into())
            }
        }
    }

    /// Queue a pane close for a session with no daemon record.
    ///
    /// `paint_dock` checks the workspace out of `layouts`. `remove_tab`
    /// only walks `layouts`, so a direct close from the pane body misses
    /// the visible tab and it reappears on check-in.
    pub(super) fn queue_unavailable_tab_close(&mut self, sid: &str) {
        self.pending_unavailable_close.push(sid.to_owned());
    }

    pub(super) fn drain_pending_unavailable_close(&mut self) {
        let pending = std::mem::take(&mut self.pending_unavailable_close);
        for sid in &pending {
            self.remove_tab(sid);
        }
    }

    pub(super) fn close_unavailable_tabs(&mut self) {
        let missing = self.unavailable_tabs();
        if missing.is_empty() {
            return;
        }
        for sid in &missing {
            self.remove_tab(sid);
        }
        self.info = Some(match missing.len() {
            1 => "Closed 1 tab without a session record.".into(),
            n => format!("Closed {n} tabs without a session record."),
        });
    }

    pub(super) fn installation_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Installation");
        if !self.connected {
            ui.label(
                "The session service is unreachable. Start it to show sessions and enable repair.",
            );
            self.start_service_button(ui, false);
            return;
        }
        if !self.state.generations.is_empty() {
            if let Some(error) = self.error.as_ref().filter(|e| {
                e.starts_with("Service upgrade") || e.starts_with("Could not repair installation")
            }) {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                if ui
                    .add_enabled(
                        !self.repair_pending,
                        egui::Button::new("Retry service update"),
                    )
                    .clicked()
                {
                    self.begin_installation_repair();
                }
            } else {
                ui.label("Updated service ready");
            }
            if self
                .state
                .generations
                .iter()
                .any(|g| g.owner.id != self.state.generation && g.live_sessions > 0)
            {
                ui.label("Existing sessions continue on an earlier version.");
            }
            let generations = self.state.generations.clone();
            for generation in generations {
                ui.collapsing(
                    format!(
                        "{} · {:?} · {} live sessions",
                        generation.owner.version, generation.owner.status, generation.live_sessions
                    ),
                    |ui| {
                        ui.monospace(&generation.owner.id);
                        ui.label(format!(
                            "Build {} · protocol {} · catalog {}",
                            generation.owner.build,
                            generation.owner.protocol,
                            generation.owner.catalog
                        ));
                        if let Some(error) = &generation.error {
                            ui.colored_label(appearance::color(&self.theme.status_failed), error);
                        }
                        let sessions: Vec<_> = self
                            .state
                            .sessions
                            .iter()
                            .filter(|s| s.generation == generation.owner.id && s.lifecycle.live())
                            .cloned()
                            .collect();
                        for session in sessions {
                            if ui.button(format!("Show {}", session.label)).clicked() {
                                self.go_session(&session.id);
                            }
                        }
                    },
                );
            }
            self.restart_session_button(ui, false);
        }
        let protected = self
            .state
            .capabilities
            .iter()
            .any(|c| c == STABLE_HELPER_CAPABILITY);
        let problem = self.installation_problem();
        let live: Vec<_> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .cloned()
            .collect();
        if problem {
            ui.colored_label(
                appearance::color(&self.theme.status_failed),
                "The session service has lost its terminal helper.",
            );
            ui.label("Installing a new app leaves the old session service running. Finish the sessions below, then repair to start the service from this installation.");
        } else if !protected {
            ui.label("The session service is still using helpers from an older installation.");
            ui.label("Finish the sessions below, then repair to protect new sessions from app moves and updates.");
        } else {
            ui.label("Installation is healthy. Terminal sessions stay available when the app is updated or moved.");
        }
        if problem || !protected || can_retire_daemon(&self.state) {
            ui.add_space(8.0);
            ui.label(match live.len() {
                0 => "No live sessions.".into(),
                1 => "1 live session must finish before repair.".into(),
                n => format!("{n} live sessions must finish before repair."),
            });
            ui.weak("Save unsaved files and close each session normally. Choosing Run in background keeps a session live.");
            for session in &live {
                ui.horizontal(|ui| {
                    let project = self
                        .state
                        .projects
                        .iter()
                        .find(|p| p.id == session.project_id)
                        .map(|p| p.name.as_str())
                        .unwrap_or("Project");
                    ui.label(format!("{project} · {}", session.label));
                    if ui.small_button("Show session").clicked() {
                        self.go_session(&session.id);
                    }
                });
            }
            let safe_restart = self
                .state
                .capabilities
                .iter()
                .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY);
            if !safe_restart {
                ui.label("This older service needs a manual restart. You can run the command below instead of logging out.");
                ui.label("1. Save your work and close every session listed above.");
                ui.label(if cfg!(target_os = "macos") {
                    "2. Copy the command, then fully quit Terminator with ⌘Q. Wait until it closes."
                } else {
                    "2. Copy the command, then fully quit Terminator. Wait until it closes."
                });
                ui.label(if cfg!(target_os = "macos") {
                    "3. Open Terminal.app and paste and run the command there."
                } else {
                    "3. Open another terminal application and paste and run the command there."
                });
                ui.label("4. After it returns \"Ok\", wait two seconds and reopen Terminator.");
                match std::env::current_exe()
                    .map_err(anyhow::Error::from)
                    .and_then(|exe| installation::manual_shutdown_command(&exe, &self.paths, false))
                {
                    Ok(command) => {
                        ui.add(
                            egui::Label::new(RichText::new(&command).monospace())
                                .wrap()
                                .selectable(true),
                        );
                        let copy = ui
                            .add_enabled(
                                live.is_empty(),
                                egui::Button::new("Copy shutdown command"),
                            )
                            .on_disabled_hover_text(
                                "Finish all live sessions before copying the recovery command.",
                            );
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "copy-shutdown-command", copy.rect);
                        if copy.clicked() {
                            ui.ctx().copy_text(command);
                            self.info = Some("Shutdown command copied. Fully quit Terminator before running it in another terminal app.".into());
                        }
                    }
                    Err(error) => {
                        ui.label(format!("Could not prepare the command: {error}"));
                    }
                }
                ui.weak("If the command reports running sessions, close them normally and retry. It preserves saved history; output already lost cannot be recovered. You can also log out and back in after saving and closing your sessions.");
            } else if live.is_empty() && !can_retire_daemon(&self.state) {
                ui.label("This service's version is newer or cannot be verified. Open the matching or newer Terminator app to repair it.");
            }
            self.restart_session_button(ui, false);
            let repair = ui.add_enabled(
                !self.repair_pending && can_retire_daemon(&self.state),
                egui::Button::new(if self.repair_pending {
                    "Repairing…"
                } else {
                    "Repair installation"
                }),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "repair-installation", repair.rect);
            if repair.clicked() {
                self.begin_installation_repair();
            }
            if let Some(error) = self
                .error
                .as_deref()
                .filter(|e| !installation::is_helper_error(e))
            {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
        }
        ui.collapsing("Installation details", |ui| {
            ui.label(format!("App: {}", env!("CARGO_PKG_VERSION")));
            ui.label(format!(
                "Session service: {}",
                self.state
                    .daemon_version
                    .as_deref()
                    .unwrap_or("not reported")
            ));
            for (label, path) in [
                ("Service", self.state.daemon_executable.as_ref()),
                ("Helper", self.state.attachment_helper_executable.as_ref()),
            ] {
                if let Some(path) = path {
                    ui.label(format!("{label}: {}", path.display()));
                }
            }
            if let Some(error) = &self.installation_error {
                ui.label(error);
            }
        });
        if !live.is_empty() {
            let _cleanup = ui.collapsing("Stop all sessions and shut down", |ui| {
                ui.label("Save your work first. This command quits Terminator, terminates every running terminal and editor, then flushes saved history and shuts down the service. Unsaved editor buffers will be lost and running jobs will stop.");
                ui.label(if cfg!(target_os = "macos") {
                    "Run it in Terminal.app. After it completes, reopen Terminator."
                } else {
                    "Run it in another terminal application. After it completes, reopen Terminator."
                });
                match std::env::current_exe().map_err(anyhow::Error::from)
                    .and_then(|exe| installation::manual_shutdown_command(&exe, &self.paths, true)) {
                    Ok(command) => {
                        ui.add(egui::Label::new(RichText::new(&command).monospace()).wrap().selectable(true));
                        if ui.button("Copy stop-all command").clicked() {
                            ui.ctx().copy_text(command);
                            self.info = Some("Stop-all command copied. Save your work before running it in another terminal app.".into());
                        }
                    }
                    Err(error) => { ui.label(format!("Could not prepare the command: {error}")); }
                }
            });
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                "show-stop-all-command",
                _cleanup.header_response.rect,
            );
        }
    }
}
