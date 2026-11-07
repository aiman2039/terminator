//! Status notices: activation focus requests and reported errors.

use crate::*;

impl App {
    /// An OS notification click goes straight to the agent's terminal
    /// (#79). The detail card stays for a notice whose session has ended or
    /// is not in the snapshot yet.
    pub(super) fn handle_activation(&mut self, ctx: &egui::Context, notice: String) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        let live = self
            .state
            .notifications
            .iter()
            .find(|candidate| candidate.id == notice)
            .and_then(|candidate| {
                self.state
                    .sessions
                    .iter()
                    .find(|session| session.id == candidate.session_id)
            })
            .is_some_and(|session| session.lifecycle.live());
        if live {
            self.apply_notice_action(notice, AttentionAction::Go);
        } else {
            self.detail = Some(notice);
        }
    }

    pub(super) fn handle_error(&mut self, e: String) {
        let kind = StatusKind::of_report(&e);
        if kind == StatusKind::Helper {
            self.installation_error = Some(e.clone());
        }
        if self.exit.active() {
            self.cancel_exit(e.clone());
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture error {e}"));
        }
        if kind == StatusKind::Connection {
            self.connected = false;
        }
        self.report_status_error(e);
    }
}
