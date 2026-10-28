//! Recovery uses the ordinary session navigation and safe idle restart paths.
mod settings;
#[cfg(test)]
mod tests;

use crate::*;

impl App {
    pub(crate) fn installation_problem(&self) -> bool {
        self.installation_error.is_some() || self.state.attachment_helper_available == Some(false)
    }

    pub(crate) fn open_installation_settings(&mut self) {
        self.open_settings();
        self.settings_section = SettingsSection::Updates;
    }

    pub(crate) fn maybe_upgrade_idle_daemon(&mut self) {
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

    pub(crate) fn restart_session_button(&mut self, ui: &mut egui::Ui, small: bool) {
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

    pub(crate) fn begin_session_restart(&mut self) {
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
            Err(()) => {
                self.error =
                    Some("Restart worker disconnected. Reopen Terminator to retry.".into());
            }
        }
    }

    pub(crate) fn begin_installation_repair(&mut self) {
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
                    Err(()) => {
                        self.error = Some(
                            "Installation worker disconnected. Reopen Terminator to retry.".into(),
                        );
                    }
                }
            }
            Err(error) => {
                self.error = Some(format!("Could not save workspace before repair: {error:#}"));
            }
        }
    }

    /// The GUI has loaded daemon state before but the snapshot poller can no
    /// longer reach the session service (reboot, crash, or a wiped runtime
    /// directory). Session records and window-close persistence are stale
    /// until the service is started again.
    pub(crate) fn service_disconnected(&self) -> bool {
        self.state_loaded && !self.connected
    }

    /// Terminal tabs without a matching session record in the latest daemon
    /// state. Only reported while connected so a stale snapshot never prunes
    /// tabs whose sessions may still exist.
    pub(crate) fn unavailable_tabs(&self) -> Vec<String> {
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
                    .flat_map(egui_dock::DockState::iter_all_tabs),
            )
            .filter_map(|(_, tab)| match tab {
                Tab::Terminal(sid) => {
                    (!self.state.sessions.iter().any(|s| s.id == *sid)).then(|| sid.clone())
                }
                _ => None,
            })
            .chain(self.floating.iter().flat_map(|window| {
                window
                    .dock
                    .as_ref()
                    .map(|dock| dock.iter_all_tabs())
                    .into_iter()
                    .flatten()
                    .filter_map(|(_, tab)| match tab {
                        Tab::Terminal(sid) if !self.state.sessions.iter().any(|s| s.id == *sid) => {
                            Some(sid.clone())
                        }
                        _ => None,
                    })
            }))
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    pub(crate) fn start_service_button(&mut self, ui: &mut egui::Ui, small: bool) {
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

    pub(crate) fn begin_service_start(&mut self) {
        if self.service_start_pending || self.connected {
            return;
        }
        match self.jobs.send(Job::StartSessionService) {
            Ok(()) => self.service_start_pending = true,
            Err(()) => {
                self.error =
                    Some("Service worker disconnected. Reopen Terminator to retry.".into());
            }
        }
    }

    /// Queue a pane close for a session with no daemon record.
    ///
    /// `paint_dock` checks the workspace out of `layouts`. `remove_tab`
    /// only walks `layouts`, so a direct close from the pane body misses
    /// the visible tab and it reappears on check-in.
    pub(crate) fn queue_unavailable_tab_close(&mut self, sid: &str) {
        self.pending_unavailable_close.push(sid.to_owned());
    }

    pub(crate) fn drain_pending_unavailable_close(&mut self) {
        let pending = std::mem::take(&mut self.pending_unavailable_close);
        for sid in &pending {
            self.remove_tab(sid);
        }
    }

    pub(crate) fn close_unavailable_tabs(&mut self) {
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

    /// Drop terminal tabs whose session record is gone, without a message.
    ///
    /// Runs after state updates so a tab left behind by a pruned (ended,
    /// non-resumable) session disappears on its own. The explicit
    /// [`Self::close_unavailable_tabs`] remains for the Info action.
    pub(crate) fn prune_unavailable_tabs(&mut self) {
        if !self.connected {
            return;
        }
        for sid in self.unavailable_tabs() {
            self.remove_tab(&sid);
        }
    }
}
