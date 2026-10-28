use anyhow::{Context, Result, ensure};
use std::time::Duration;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;

impl Shared {
    pub(super) fn on_editor_save(&self, session: &str) -> Result<Outcome> {
        self.editor_rpc(session, "execute('wall')")
            .map(Outcome::Reply)
    }

    pub(super) fn on_editor_compare(&self, session: &str) -> Result<Outcome> {
        self.editor_rpc(session, "execute('TerminatorCompareDisk')")
            .map(Outcome::Reply)
    }

    pub(super) fn on_editor_status(&self, session: &str) -> Result<Outcome> {
        self.editor_rpc(
            session,
            "string(len(filter(getbufinfo(), 'v:val.changed')))",
        )
        .map(Outcome::Reply)
    }

    fn editor_rpc(&self, sid: &str, expression: &str) -> Result<Response> {
        let state = relock(&self.state);
        if let Some(remote) = state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .and_then(|s| s.remote.clone())
        {
            drop(state);
            return Ok(Response::Text(String::from_utf8(
                crate::remote::terminal::editor_rpc(
                    &remote,
                    self.ssh_control(&remote.ssh).as_deref(),
                    expression,
                )?,
            )?));
        }

        let program = if state.sessions.iter().any(|s| s.id == sid && s.review) {
            "nvim".to_owned()
        } else {
            state.settings.editor_program.clone()
        };
        drop(state);
        let socket = self.paths.editor_socket(sid);
        // Neovim listens on a named pipe on Windows, so the socket path has
        // no filesystem record there; the `--server` address must match the
        // `--listen` address from `editor::prepare` on every platform.
        let server = transport::nvim_listen_arg(&socket);
        #[cfg(unix)]
        ensure!(socket.exists(), "Embedded editor RPC unavailable");
        let editor = find_executable(&program).context("Editor executable not found")?;
        let mut command = std::process::Command::new(editor);
        command
            .args(["--server"])
            .arg(server)
            .args(["--remote-expr", expression]);
        let out = bounded_output(command, Duration::from_secs(2))?;
        ensure!(
            out.status.success(),
            "Editor request failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(Response::Text(
            String::from_utf8_lossy(&out.stdout).trim().into(),
        ))
    }
}
