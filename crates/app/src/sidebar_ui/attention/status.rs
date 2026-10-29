//! Notification list, resource status, and the status badge.
use super::*;

impl App {
    pub(crate) fn notifications(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let pending = self
                .state
                .notifications
                .iter()
                .filter(|n| notice_pending(n, now()))
                .count()
                .saturating_add(
                    self.state
                        .terminal_notices
                        .iter()
                        .filter(|n| !n.dismissed)
                        .count(),
                );
            let label = if pending == 0 {
                String::new()
            } else {
                pending.to_string()
            };
            let button = egui::Button::image_and_text(
                egui::Image::new(icons::source("Bell")).fit_to_exact_size(egui::vec2(16.0, 16.0)),
                label,
            )
            .frame(false);
            let response = ui
                .add(button)
                .on_hover_text(format!("{pending} pending notifications"));
            if response.clicked() {
                self.preferences.agents_tab = AgentsTab::NeedsAttention;
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "attention-bell", response.rect);
            let popup = egui::Popup::from_toggle_button_response(&response).show(|ui| {
                ui.set_width(300.0);
                ui.set_max_height(460.0);
                ui.push_id("attention-popup", |ui| self.agents_view(ui));
            });
            #[cfg(feature = "test-support")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::unique("attention-state"),
                    (pending, popup.is_some()),
                )
            });
            let _ = popup;
        });
    }

    /// IDE status-bar mirror of the left agent bell: always visible in IDE
    /// mode so waiting/unread counts survive collapsed sidebars. Clicking
    /// reveals the Agents inbox in the right sidebar.
    /// Right end of the bottom status strip: app CPU and memory across the
    /// GUI, daemon, and hook processes. Pure paint over the latest background
    /// sample (one every few seconds), so it schedules no repaints of its
    /// own — the strip already repaints on the one-second heartbeat.
    /// Clicking opens the Info tool, which breaks the same sample down per
    /// session and system.
    pub(crate) fn app_resource_status(&mut self, ui: &mut egui::Ui) {
        let Some(sample) = &self.resources else {
            return;
        };
        let total = sample.app.total();
        // The GUI process itself must always match; zero means the name
        // heuristic failed on this platform, and a stuck "0 MB" would be
        // worse than no readout.
        if total.processes == 0 {
            return;
        }
        let component = |name: &str, stats: crate::resource_sample::ComponentStats| {
            format!(
                "{} ({}): {} · {}",
                name,
                stats.processes,
                session_info::format_percent(stats.cpu),
                session_info::format_bytes(stats.memory)
            )
        };
        let label = format!(
            "{} · {}",
            session_info::format_percent(total.cpu),
            session_info::format_bytes(total.memory)
        );
        let response = ui
            .add(egui::Label::new(RichText::new(label).small().weak()).sense(egui::Sense::click()))
            .on_hover_text(format!(
                "Terminator app processes ({})\n{}\n{}\n{}\n\nClick to open Info → Resources.",
                total.processes,
                component("GUI", sample.app.gui),
                component("Daemon", sample.app.daemon),
                component("Hooks", sample.app.hooks),
            ))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-resources", response.rect);
        if response.clicked() {
            self.preferences.tool = SidebarTool::Info;
            self.preferences.visible = true;
        }
    }

    pub(crate) fn notification_status_badge(&mut self, ui: &mut egui::Ui) {
        let (waiting, unread) = self.attention_counts();
        let label = attention_badge_label(waiting, unread);
        let response = ui
            .add(
                egui::Button::image_and_text(
                    egui::Image::new(icons::source("Bell"))
                        .fit_to_exact_size(egui::vec2(14.0, 14.0)),
                    label,
                )
                .frame(false),
            )
            .on_hover_text("Pending agent notifications. Click to open the Agents inbox.");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-attention-bell", response.rect);
        if response.clicked() {
            self.open_agents_inbox();
        }
    }
}
