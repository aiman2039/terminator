//! Ports tool: listening TCP ports of the selected project's local sessions,
//! with Open, Copy URL, Go to session and a confirmed Stop.
use crate::ports_refresh::{PortsUpdate, Request};
use crate::*;
use terminator_core::metadata::SessionPort;

/// GUI state of the Ports tool. The worker scans only while the tool is
/// drawn; a frame without it sends `None` and the scan stops.
#[derive(Default)]
pub(crate) struct PortsTool {
    drawn: bool,
    request: Option<Request>,
    generation: u64,
    listed: Option<Result<Vec<SessionPort>, String>>,
    /// The row whose Stop waits for confirmation.
    confirm: Option<SessionPort>,
}

impl App {
    /// Live local sessions of the selected project that have a process.
    fn port_sessions(&self) -> Vec<(String, (u32, u64))> {
        let Some(project) = self.selected.as_ref() else {
            return Vec::new();
        };
        self.state
            .sessions
            .iter()
            .filter(|s| &s.project_id == project && s.remote.is_none() && s.lifecycle.live())
            .filter_map(|s| s.pid.map(|pid| (s.id.clone(), (pid, s.created))))
            .collect()
    }

    pub(crate) fn sync_ports_request(&mut self) {
        let drawn = std::mem::take(&mut self.ports.drawn);
        let sessions = if drawn {
            self.port_sessions()
        } else {
            Vec::new()
        };
        let next = (drawn && !sessions.is_empty()).then_some(sessions);
        if next.as_ref() == self.ports.request.as_ref().map(|r| &r.sessions) {
            return;
        }
        self.ports.generation = self.ports.generation.wrapping_add(1);
        self.ports.listed = next.as_ref().map_or(Some(Ok(Vec::new())), |_| None);
        self.ports.request = next.map(|sessions| Request {
            sessions,
            generation: self.ports.generation,
        });
        let _ = self.ports_jobs.list.send(self.ports.request.clone());
    }

    pub(crate) fn handle_ports(&mut self, update: PortsUpdate) {
        match update {
            PortsUpdate::Listed(generation, listed) => {
                if generation == self.ports.generation {
                    self.ports.listed = Some(listed);
                }
            }
            PortsUpdate::Stopped(port, Ok(())) => {
                self.info = Some(format!("Sent stop to {} (pid {}).", port.command, port.pid));
            }
            PortsUpdate::Stopped(port, Err(error)) => {
                self.error
                    .set(format!("Could not stop port {}: {error}", port.port));
            }
        }
    }

    pub(crate) fn ports_panel(&mut self, ui: &mut egui::Ui) {
        self.ports.drawn = true;
        ui.strong("Ports");
        ui.add_space(4.0);
        if self.selected_remote_project().is_some() {
            ui.weak("Ports lists local terminals only. Use the Hosts tool for SSH port forwards.");
            return;
        }
        let rows = match &mut self.ports.listed {
            None => {
                ui.weak("Looking for listening ports…");
                return;
            }
            Some(Err(error)) => {
                ui.weak(format!("Could not list ports: {error}"));
                return;
            }
            Some(Ok(rows)) if rows.is_empty() => {
                ui.weak("No listening ports in this project's terminals.");
                return;
            }
            Some(Ok(rows)) => std::mem::take(rows),
        };
        appearance::sidebar_scroll("ports-tool").show(ui, |ui| {
            for port in &rows {
                self.port_row(ui, port);
                ui.add_space(4.0);
            }
        });
        // Taken only to draw without a clone; nothing changes them meanwhile.
        self.ports.listed = Some(Ok(rows));
    }

    fn session_label(&self, sid: &str) -> String {
        self.state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .map_or_else(|| "Terminal".to_owned(), |s| s.label.clone())
    }

    fn port_row(&mut self, ui: &mut egui::Ui, port: &SessionPort) {
        let url = port.url();
        ui.horizontal(|ui| {
            if ui
                .link(format!(":{}", port.port))
                .on_hover_text(&port.address)
                .clicked()
            {
                self.open_port_tab(&url);
            }
            ui.label(format!("{} ({})", port.command, port.pid));
        });
        ui.horizontal(|ui| {
            ui.weak(self.session_label(&port.session));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.port_actions(ui, port, &url);
            });
        });
        if self.ports.confirm.as_ref() == Some(port) {
            self.confirm_stop(ui, port);
        }
    }

    fn port_actions(&mut self, ui: &mut egui::Ui, port: &SessionPort, url: &str) {
        if appearance::sidebar_action(ui, "CircleStop", "Stop process…").clicked() {
            self.ports.confirm = Some(port.clone());
        }
        if appearance::sidebar_action(ui, "LocateFixed", "Go to terminal").clicked() {
            self.go_session(&port.session);
        }
        if appearance::sidebar_action(ui, "Copy", "Copy URL").clicked() {
            ui.ctx().copy_text(url.to_owned());
            self.info = Some(format!("Copied {url}"));
        }
        if appearance::sidebar_action(ui, "ExternalLink", "Open in system browser").clicked() {
            let _ = self.jobs.send(Job::Browser(url.to_owned()));
        }
        if appearance::sidebar_action(ui, "Globe", "Open in Browser tab").clicked() {
            self.open_port_tab(url);
        }
    }

    /// Opens `url` in a Browser tab of the selected project.
    pub(crate) fn open_port_tab(&mut self, url: &str) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        if let Err(error) = self.open_browser_url(&project, url, None) {
            self.error.set(format!("Could not open {url}: {error:#}"));
        }
    }

    fn confirm_stop(&mut self, ui: &mut egui::Ui, port: &SessionPort) {
        ui.label(format!("Stop {} (pid {})?", port.command, port.pid));
        ui.horizontal(|ui| {
            if appearance::danger_button(ui, "Stop").clicked() {
                self.ports.confirm = None;
                let _ = self.ports_jobs.stop.send(port.clone());
            }
            if ui.button("Cancel").clicked() {
                self.ports.confirm = None;
            }
        });
    }
}

#[cfg(test)]
mod tests;
