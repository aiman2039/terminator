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

    /// Whether the quit must first ask about live VNC sessions: once per
    /// quit. An app restart (for example an update) does not ask: the
    /// daemon keeps the sessions and the new GUI reattaches them.
    pub(crate) fn vnc_quit_needs_prompt(&self) -> bool {
        // A native fixture closes its own window and cannot answer: treat
        // the close as Quit, which keeps the sessions.
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            return false;
        }
        !self.vnc_quit_confirmed
            && self.service_op != crate::ServiceOp::Restarting
            && !self.live_vnc_sessions().is_empty()
    }

    /// The in-app question shown while a quit waits on live VNC sessions.
    pub(crate) fn vnc_quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.vnc_quit_prompt {
            return;
        }
        let count = self.live_vnc_sessions().len();
        let window = egui::Window::new("Quit Terminator")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO);
        let dialog = appearance::Confirm::new(
            format!(
                "{} {} connected after Terminator closes.",
                appearance::count_noun(count, "VNC session"),
                if count == 1 { "stays" } else { "stay" }
            ),
            "",
        )
        .destructive(appearance::ConfirmButton::new("Disconnect all and quit"))
        .default_action(appearance::ConfirmButton::new("Quit"));
        let choice = appearance::confirm(ctx, window, dialog, |_| {}).map(|answer| match answer {
            appearance::ConfirmAnswer::Default => QuitChoice::Keep,
            appearance::ConfirmAnswer::Destructive => QuitChoice::Disconnect,
            appearance::ConfirmAnswer::Cancel => QuitChoice::Cancel,
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
