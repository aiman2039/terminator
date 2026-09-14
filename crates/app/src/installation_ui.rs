//! Recovery uses the ordinary session navigation and safe idle restart paths.
use super::*;

impl App {
    pub(super) fn installation_problem(&self) -> bool {
        self.installation_error.is_some() || self.state.attachment_helper_available == Some(false)
    }

    pub(super) fn open_installation_settings(&mut self) {
        self.open_settings();
        self.settings_section = 6;
    }

    pub(super) fn begin_installation_repair(&mut self) {
        if self.repair_pending || !self.connected || !can_retire_daemon(&self.state) {
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

    pub(super) fn installation_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Installation");
        if !self.connected {
            ui.label(
                "Waiting for the session service. Repair will be available after it reconnects.",
            );
            return;
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
                        self.settings_open = false;
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
                ui.label("This older service cannot restart safely from the app. After saving your work and closing all sessions, log out of your OS account and back in once, then open the installed Terminator app.");
            } else if live.is_empty() && !can_retire_daemon(&self.state) {
                ui.label("This service's version is newer or cannot be verified. Open the matching or newer Terminator app to repair it.");
            }
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
    }
}
