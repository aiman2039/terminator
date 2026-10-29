//! The VNC page: the address book filling the main pane between the
//! sidebars, like Settings and Player. Opening the VNC tool shows it;
//! connecting, or switching to a workspace tab, hides it.
use super::*;

/// View-state key for the page's address bar.
pub(super) const PAGE_KEY: &str = "vnc-page";

impl App {
    /// Show the VNC page in the main pane, replacing any other center view.
    pub(crate) fn show_vnc_page(&mut self) {
        self.hide_center_overlay();
        self.vnc_page_open = true;
    }

    /// The main pane: the VNC page while it is open, else the workspace.
    pub(crate) fn vnc_page_or_workspace_center(&mut self, ui: &mut egui::Ui) {
        if !self.vnc_page_open {
            self.workspace_center(ui);
            return;
        }
        egui::Frame::NONE.inner_margin(16.0).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("VNC").size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if appearance::sidebar_action(ui, "X", "Close").clicked() {
                        self.vnc_page_open = false;
                    }
                });
            });
            ui.add_space(8.0);
            self.vnc_address_book(ui, PAGE_KEY);
        });
    }
}
