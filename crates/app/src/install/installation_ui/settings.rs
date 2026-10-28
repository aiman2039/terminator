//! Settings → Updates → Installation: service owners, helper health, repair.
use crate::*;

impl App {
    pub(crate) fn installation_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Installation");
        if !self.connected {
            ui.label(
                "The session service is unreachable. Start it to show sessions and enable repair.",
            );
            self.start_service_button(ui, false);
            return;
        }
        if !self.state.generations.is_empty() {
            self.service_owners(ui);
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
        self.helper_status(ui, problem, protected);
        if problem || !protected || can_retire_daemon(&self.state) {
            self.repair_section(ui, &live);
        }
        self.installation_details(ui);
        if !live.is_empty() {
            self.stop_all_command(ui);
        }
    }

    fn service_owners(&mut self, ui: &mut egui::Ui) {
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
        let generations: Vec<_> = self
            .state
            .generations
            .iter()
            .filter(|generation| {
                generation.owner.id == self.state.generation
                    || generation.live_sessions > 0
                    // An unavailable current owner can have an incomplete
                    // inventory, so keep its actionable diagnostic visible.
                    || (generation.error.is_some()
                        && generation.owner.status
                            == terminator_core::generations::Status::Draining)
            })
            .cloned()
            .collect();
        for generation in generations {
            self.service_owner_row(ui, &generation);
        }
        self.restart_session_button(ui, false);
    }

    fn service_owner_row(
        &mut self,
        ui: &mut egui::Ui,
        generation: &terminator_core::generations::Health,
    ) {
        let sessions_label = if generation.error.is_some() && generation.live_sessions == 0 {
            "session status unavailable".to_string()
        } else {
            format!("{} live sessions", generation.live_sessions)
        };
        ui.collapsing(
            format!(
                "{} · {:?} · {sessions_label}",
                generation.owner.version, generation.owner.status
            ),
            |ui| {
                ui.monospace(&generation.owner.id);
                ui.label(format!(
                    "Build {} · protocol {} · catalog {}",
                    generation.owner.build, generation.owner.protocol, generation.owner.catalog
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

    fn helper_status(&self, ui: &mut egui::Ui, problem: bool, protected: bool) {
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
    }

    fn repair_section(&mut self, ui: &mut egui::Ui, live: &[Session]) {
        ui.add_space(8.0);
        ui.label(match live.len() {
            0 => "No live sessions.".into(),
            1 => "1 live session must finish before repair.".into(),
            n => format!("{n} live sessions must finish before repair."),
        });
        ui.weak("Save unsaved files and close each session normally. Choosing Run in background keeps a session live.");
        for session in live {
            self.live_session_row(ui, session);
        }
        let safe_restart = self
            .state
            .capabilities
            .iter()
            .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY);
        if !safe_restart {
            self.manual_shutdown_steps(ui, live.is_empty());
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

    fn live_session_row(&mut self, ui: &mut egui::Ui, session: &Session) {
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

    /// Older services lack shutdown-if-idle: walk the user through a manual
    /// restart. Copying is only enabled once no session is live.
    fn manual_shutdown_steps(&mut self, ui: &mut egui::Ui, idle: bool) {
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
            Ok(command) => self.copy_shutdown_command(ui, command, idle),
            Err(error) => {
                ui.label(format!("Could not prepare the command: {error}"));
            }
        }
        ui.weak("If the command reports running sessions, close them normally and retry. It preserves saved history; output already lost cannot be recovered. You can also log out and back in after saving and closing your sessions.");
    }

    fn copy_shutdown_command(&mut self, ui: &mut egui::Ui, command: String, idle: bool) {
        ui.add(
            egui::Label::new(RichText::new(&command).monospace())
                .wrap()
                .selectable(true),
        );
        let copy = ui
            .add_enabled(idle, egui::Button::new("Copy shutdown command"))
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

    fn installation_details(&self, ui: &mut egui::Ui) {
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
    }

    fn stop_all_command(&mut self, ui: &mut egui::Ui) {
        let _cleanup = ui.collapsing("Stop all sessions and shut down", |ui| {
            ui.label("Save your work first. This command quits Terminator, terminates every running terminal and editor, then flushes saved history and shuts down the service. Unsaved editor buffers will be lost and running jobs will stop.");
            ui.label(if cfg!(target_os = "macos") {
                "Run it in Terminal.app. After it completes, reopen Terminator."
            } else {
                "Run it in another terminal application. After it completes, reopen Terminator."
            });
            match std::env::current_exe()
                .map_err(anyhow::Error::from)
                .and_then(|exe| installation::manual_shutdown_command(&exe, &self.paths, true))
            {
                Ok(command) => {
                    ui.add(
                        egui::Label::new(RichText::new(&command).monospace())
                            .wrap()
                            .selectable(true),
                    );
                    if ui.button("Copy stop-all command").clicked() {
                        ui.ctx().copy_text(command);
                        self.info = Some("Stop-all command copied. Save your work before running it in another terminal app.".into());
                    }
                }
                Err(error) => {
                    ui.label(format!("Could not prepare the command: {error}"));
                }
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
