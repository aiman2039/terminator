//! Status notices: activation focus requests and reported errors.

use crate::*;

impl App {
    pub(super) fn handle_activation(&mut self, ctx: &egui::Context, notice: String) {
        self.detail = Some(notice);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(super) fn handle_error(&mut self, e: String) {
        if installation::is_helper_error(&e) {
            self.installation_error = Some(e.clone());
        }
        if self.exit.active() {
            self.cancel_exit(e.clone());
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture error {e}"));
        }
        if daemon_connection::is_connection_error(&e) {
            self.connected = false;
        }
        self.report_status_error(e);
    }
}
