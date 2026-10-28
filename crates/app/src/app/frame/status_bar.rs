//! Bottom status bar: connection, the highest-priority banner, branch and
//! PR metadata, breadcrumb, diagnostics, and the right-aligned readouts.

use crate::*;

impl App {
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            self.connection_status(ui);
            ui.separator();
            self.status_banner(ui);
            self.status_metadata(ui);
            self.status_context(ui);
            if self.preferences.ide_mode {
                ui.separator();
                self.player_status_row(ui);
            }
            self.notification_status_badge(ui);
            // Single right-aligned block (see status_right_end): resource
            // readout, sidebar-height toggle, then the strip collapse.
            // Right-aligned app totals. Pure paint over the background
            // sample: no extra wakes, updates land with the heartbeat.
            self.status_right_end(ui);
        });
    }

    fn connection_status(&self, ui: &mut egui::Ui) {
        ui.colored_label(
            appearance::color(if self.connected {
                &self.theme.status_running
            } else {
                &self.theme.status_waiting
            }),
            if self.connected {
                "● Connected"
            } else {
                "○ Connecting"
            },
        )
        .on_hover_text(format!(
            "GUI {}\nDaemon {}\nDaemon executable: {}\nAttachment helper: {}",
            env!("CARGO_PKG_VERSION"),
            self.state
                .daemon_version
                .as_deref()
                .unwrap_or("unknown (older daemon)"),
            self.state
                .daemon_executable
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not reported by this daemon".into()),
            match self.state.attachment_helper_available {
                Some(true) => "available",
                Some(false) => "unavailable",
                None => "not reported by this daemon",
            }
        ));
    }

    /// The single most urgent status message, falling back to the count
    /// of running sessions.
    fn status_banner(&mut self, ui: &mut egui::Ui) {
        // A count only: holding the tab list across the chain would
        // borrow self while the banner bodies need `&mut self`.
        let unavailable_tabs = self.unavailable_tabs().len();
        if self.installation_problem() {
            self.installation_repair_banner(ui);
        } else if self.service_disconnected() {
            ui.horizontal_wrapped(|ui| self.service_unreachable_banner(ui));
        } else if let Some(error) = self.error.clone() {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Dismiss").clicked() {
                    self.dismiss_status_error();
                }
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            });
        } else if unavailable_tabs > 0 {
            ui.horizontal_wrapped(|ui| self.unavailable_tabs_banner(ui, unavailable_tabs));
        } else if let Some(message) = &self.state.degraded {
            ui.colored_label(appearance::color(&self.theme.status_waiting), message);
        } else if self.installation_mismatched() {
            if ui.small_button("Review installation…").clicked() {
                self.open_installation_settings();
            }
            self.restart_session_button(ui, true);
            ui.weak("App and session service use different installations.");
        } else if let Some(info) = self.info.clone() {
            ui.horizontal(|ui| {
                if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                    self.info = None;
                }
                ui.label(info);
            });
        } else {
            ui.horizontal(|ui| self.running_sessions_label(ui));
        }
    }

    fn installation_repair_banner(&mut self, ui: &mut egui::Ui) {
        let repair = ui.small_button("Fix installation…");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "fix-installation", repair.rect);
        if repair.clicked() {
            self.open_installation_settings();
        }
        self.restart_session_button(ui, true);
        ui.colored_label(
            appearance::color(&self.theme.status_failed),
            "Terminal helper needs repair. Existing sessions are preserved.",
        );
    }

    fn service_unreachable_banner(&mut self, ui: &mut egui::Ui) {
        self.start_service_button(ui, true);
        ui.colored_label(
            appearance::color(&self.theme.status_failed),
            "Session service unreachable.",
        );
        if let Some(detail) = self
            .error
            .clone()
            .filter(|error| !daemon_connection::is_connection_error(error))
        {
            ui.colored_label(appearance::color(&self.theme.status_failed), detail);
        } else {
            ui.weak("Start it to show and close sessions. Ended sessions remain in History.");
        }
    }

    fn unavailable_tabs_banner(&mut self, ui: &mut egui::Ui, unavailable_tabs: usize) {
        let close = ui.small_button("Close unavailable tabs");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "close-unavailable-tabs", close.rect);
        if close.clicked() {
            self.close_unavailable_tabs();
        }
        ui.weak(if unavailable_tabs == 1 {
            "1 tab has no session record. It was left behind by sessions that already ended.".into()
        } else {
            format!("{unavailable_tabs} tabs have no session record. They were left behind by sessions that already ended.")
        });
    }

    /// The daemon runs another version, or lacks the stable helper.
    fn installation_mismatched(&self) -> bool {
        self.state_loaded
            && (self.state.daemon_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                || !self
                    .state
                    .capabilities
                    .iter()
                    .any(|c| c == STABLE_HELPER_CAPABILITY))
    }

    fn running_sessions_label(&self, ui: &mut egui::Ui) {
        ui.weak(format!(
            "{} sessions running",
            self.state
                .sessions
                .iter()
                .filter(|s| {
                    s.lifecycle.live() && daemon_connection::session_owner_available(&self.state, s)
                })
                .count()
        ));
    }

    /// Branch or worktree, the pull request link, and up to three ports.
    fn status_metadata(&mut self, ui: &mut egui::Ui) {
        let Some(metadata) = self.metadata.clone() else {
            return;
        };
        if let Some(branch) = metadata.branch {
            ui.weak(if metadata.worktree {
                format!("Worktree · {branch}")
            } else {
                branch
            });
        }
        if let Some(pr) = metadata.pull_request
            && ui
                .link(format!("PR #{}", pr.number))
                .on_hover_text(pr.title)
                .clicked()
        {
            let _ = self.jobs.send(Job::Browser(pr.url));
        }
        for port in metadata.ports.iter().take(3) {
            if ui
                .link(format!(":{}", port.port))
                .on_hover_text(&port.address)
                .clicked()
            {
                let _ = self.jobs.send(Job::Browser(port.url()));
            }
        }
    }

    /// Status-bar context after the branch block: breadcrumb
    /// (project › active file or terminal), language-server diagnostics,
    /// and the active editor's LSP state.
    pub(crate) fn status_context(&mut self, ui: &mut egui::Ui) {
        let Some(project_id) = self.selected.clone() else {
            return;
        };
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        let pane = self
            .layouts
            .get(&project_id)
            .and_then(|workspace| workspace.active_pane().cloned());
        let editor_path = match &pane {
            Some(Tab::NativeEditor { path }) => Some(path.clone()),
            _ => None,
        };
        let leaf = self.breadcrumb_leaf(&project, pane.as_ref());
        ui.separator();
        let _crumb = ui.weak(&project.name);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-breadcrumb-project", _crumb.rect);
        if let Some(leaf) = leaf {
            ui.weak("›");
            let _leaf = ui.weak(leaf);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "status-breadcrumb", _leaf.rect);
        }
        self.status_diagnostics(ui);
        if let Some(path) = editor_path
            && self.lsp.tracked(&path)
        {
            self.status_lsp(ui, &path);
        }
    }

    /// The active file relative to the project, or the terminal's label.
    fn breadcrumb_leaf(&self, project: &Project, pane: Option<&Tab>) -> Option<String> {
        match pane {
            Some(Tab::NativeEditor { path } | Tab::Image { path }) => {
                Some(self.remote_file_label(path).unwrap_or_else(|| {
                    path.strip_prefix(&project.path).map_or_else(
                        |_| path.display().to_string(),
                        |rel| rel.display().to_string(),
                    )
                }))
            }
            Some(Tab::Terminal(sid)) => Some(
                self.state
                    .sessions
                    .iter()
                    .find(|session| &session.id == sid)
                    .map_or_else(|| "Terminal".to_string(), |session| session.label.clone()),
            ),
            _ => None,
        }
    }

    fn status_diagnostics(&mut self, ui: &mut egui::Ui) {
        let (errors, warnings, total) = self.lsp.problem_counts();
        if total == 0 {
            return;
        }
        ui.separator();
        let mut parts = Vec::new();
        if errors > 0 {
            parts.push(format!("{errors} errors"));
        }
        if warnings > 0 {
            parts.push(format!("{warnings} warnings"));
        }
        if errors == 0 && warnings == 0 {
            parts.push(format!("{total} problems"));
        }
        let diag = ui
            .colored_label(
                appearance::color(if errors > 0 {
                    &self.theme.status_failed
                } else {
                    &self.theme.status_waiting
                }),
                parts.join(" · "),
            )
            .on_hover_text(
                "Diagnostics from language servers for open files — click to open Problems",
            );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-diagnostics", diag.rect);
        if diag.interact(egui::Sense::click()).clicked() {
            self.open_problems();
        }
    }

    fn status_lsp(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        ui.separator();
        if let Some(note) = self.lsp.server_note(path) {
            let retry = ui
                .weak("LSP unavailable")
                .on_hover_text(format!("{note}\nClick to retry"))
                .interact(egui::Sense::click());
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "status-lsp", retry.rect);
            if retry.clicked() {
                self.lsp.restart(path);
            }
        } else {
            let _lsp = ui.weak("LSP ready");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "status-lsp", _lsp.rect);
        }
    }
}
