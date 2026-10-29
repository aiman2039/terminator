//! Quitting with live VNC sessions: they keep running in the daemon, so the
//! quit asks once whether to leave them connected or disconnect them.
use super::*;

impl App {
    /// Ids of VNC sessions that would stay connected after the GUI quits.
    pub(crate) fn live_vnc_sessions(&self) -> Vec<String> {
        self.state
            .vnc_sessions
            .iter()
            .filter(|s| s.status.live())
            .map(|s| s.id.clone())
            .collect()
    }

    /// Whether the quit may go on now. With live VNC sessions it first asks,
    /// once per quit.
    pub(crate) fn vnc_quit_allowed(&mut self) -> bool {
        if self.vnc_quit_confirmed || self.live_vnc_sessions().is_empty() {
            return true;
        }
        self.vnc_quit_prompt = true;
        false
    }

    /// The in-app question shown while a quit waits on live VNC sessions.
    pub(crate) fn vnc_quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.vnc_quit_prompt {
            return;
        }
        let count = self.live_vnc_sessions().len();
        let mut choice = None;
        egui::Window::new("Quit Terminator")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{count} VNC session{} stay connected after Terminator closes.",
                    if count == 1 { "" } else { "s" }
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if form::primary_button(ui, "Quit").clicked() {
                        choice = Some(QuitChoice::Keep);
                    }
                    if ui.button("Disconnect all and quit").clicked() {
                        choice = Some(QuitChoice::Disconnect);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(QuitChoice::Cancel);
                    }
                });
            });
        if let Some(choice) = choice {
            self.answer_vnc_quit(ctx, choice);
        }
    }

    pub(crate) fn answer_vnc_quit(&mut self, ctx: &egui::Context, choice: QuitChoice) {
        self.vnc_quit_prompt = false;
        if choice == QuitChoice::Cancel {
            self.cancel_app_quit();
            return;
        }
        if choice == QuitChoice::Disconnect {
            for session in self.live_vnc_sessions() {
                self.send(Request::VncDisconnect { session });
            }
        }
        self.vnc_quit_confirmed = true;
        self.pending_app_quit = true;
        self.continue_app_quit(ctx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuitChoice {
    Keep,
    Disconnect,
    Cancel,
}
