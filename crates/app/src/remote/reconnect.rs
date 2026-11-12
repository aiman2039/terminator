//! Reconnect for a plain OpenSSH terminal that lost its connection. Only an
//! explicit click opens the new terminal; it runs what a new SSH terminal of
//! the host runs (the saved Terminal command or the login shell) and takes
//! the dead pane's place.
use crate::{After, App, Job, appearance};
use eframe::egui;
use terminator_core::{Request, Session, remote::reconnect::connection_lost};

impl App {
    /// The ended session is an OpenSSH terminal whose connection dropped.
    pub(crate) fn ssh_connection_lost(&self, session: &Session) -> bool {
        connection_lost(session, &self.state.remote_projects)
    }

    /// Opens a new SSH terminal for the same host and folder; the reply
    /// moves it into every pane that showed `session`.
    pub(crate) fn reconnect_ssh(&mut self, session: &Session) {
        if !self.ssh_connection_lost(session) {
            return;
        }
        let Some(remote) = &session.remote else {
            return;
        };
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: session.project_id.clone(),
                cwd: Some(remote.cwd.clone().into()),
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            After::Restore(session.id.clone()),
        ));
        self.info = Some(format!("Reconnecting to {}…", remote.ssh.authority()));
    }

    /// The daemon keeps a dropped SSH terminal's record for Reconnect; it
    /// goes when its pane is replaced or closed.
    pub(crate) fn forget_dropped_ssh(&mut self, sid: &str) {
        let dropped = self
            .state
            .sessions
            .iter()
            .any(|s| s.id == sid && self.ssh_connection_lost(s));
        if dropped && !self.state.session_has_resume(sid) {
            self.send(Request::Remove {
                session: sid.to_owned(),
            });
        }
    }

    /// The Reconnect bar of a dead pane: shown only for a lost OpenSSH
    /// connection.
    pub(crate) fn reconnect_bar(&mut self, ui: &mut egui::Ui, session: &Session) {
        if !self.ssh_connection_lost(session) {
            return;
        }
        let host = session
            .remote
            .as_ref()
            .map(|r| r.ssh.authority())
            .unwrap_or_default();
        let mut clicked = false;
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                appearance::color(&self.theme.status_failed),
                format!("Connection to {host} was lost."),
            );
            clicked = ui
                .button("Reconnect")
                .on_hover_text(
                    "Open a new SSH terminal for this host here. It runs only the host's Terminal command.",
                )
                .clicked();
        });
        if clicked {
            self.reconnect_ssh(session);
        }
    }
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
