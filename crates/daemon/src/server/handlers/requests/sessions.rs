use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use terminator_core::*;

use super::super::super::shared::{HistoryJob, Runtime, Shared, relock};
use super::Outcome;
use crate::{signals, storage};

impl Shared {
    pub(super) fn on_prune_history(&self, budget: u64) -> Result<Outcome> {
        let (tx, rx) = mpsc::channel();
        self.history.send(HistoryJob::Prune(budget, tx))?;
        let ids = rx
            .recv_timeout(Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)?;
        let mut state = relock(&self.state);
        for session in &mut state.sessions {
            if ids.contains(&session.id) {
                session.truncated = true;
            }
        }
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_shell_command(&self, session: &str) -> Result<Outcome> {
        // Older generated shells still emit prompt hooks; ignore them.
        let _ = self.runtime(session)?;
        Ok(Outcome::Reply(Response::Text("0".into())))
    }

    pub(super) fn on_shell_prompt(&self, session: &str) -> Result<Outcome> {
        let _ = self.runtime(session)?;
        Ok(Outcome::Changed)
    }

    pub(super) fn on_screen(&self, session: &str) -> Result<Outcome> {
        let runtime = self.runtime(session)?;
        let screen = relock(&runtime).parser.screen().contents();
        Ok(Outcome::Reply(Response::Text(screen)))
    }

    pub(super) fn on_create(
        self: &Arc<Self>,
        input: super::super::sessions::CreateInput,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::Created(self.create(input)?)))
    }

    pub(super) fn on_cwd(&self, session: &str, path: PathBuf) -> Result<Outcome> {
        let _worktree_guard = relock(&self.worktree_operations);
        ensure!(path.is_absolute(), "Expected absolute cwd");
        let mut s = relock(&self.state);
        let rec = s
            .sessions
            .iter_mut()
            .find(|r| r.id == session && r.lifecycle.live())
            .context("Unknown live session")?;
        ensure!(
            rec.remote.is_none(),
            "Remote cwd updates require a remote session channel"
        );
        rec.cwd = path;
        rec.cwd_confirmed = true;
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_focus(&self, session: &str) -> Outcome {
        relock(&self.state).focus(session);
        Outcome::Changed
    }

    pub(super) fn on_rename(&self, session: &str, label: String) -> Result<Outcome> {
        ensure!(label.len() <= 256, "Label too long");
        let mut s = relock(&self.state);
        s.sessions
            .iter_mut()
            .find(|r| r.id == session)
            .context("Unknown session")?
            .label = label;
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_stop(&self, session: &str) -> Result<Outcome> {
        let _operation = relock(&self.terminal_operations);
        let rt = self.runtime(session)?;
        let mut s = relock(&self.state);
        let rec = s
            .sessions
            .iter_mut()
            .find(|r| r.id == session)
            .context("Unknown session")?;
        ensure!(!relock(&rt).ended, "Session ended");
        if let Some(pid) = rec.pid {
            Self::hang_up_session(&rt, pid)?;
        }
        rec.lifecycle = Lifecycle::Stopping;
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    fn hang_up_session(rt: &Arc<Mutex<Runtime>>, pid: u32) -> Result<()> {
        // Signal the PTY's foreground job as well as its owning shell.
        #[cfg(unix)]
        if let Ok(shell_pid) = i32::try_from(pid)
            && let Some(foreground) = relock(rt).master.process_group_leader()
            && foreground > 1
            && foreground != shell_pid
            && let Ok(foreground) = u32::try_from(foreground)
        {
            let _ = signals::signal_group(foreground, signals::ProcSignal::Hangup);
        }
        // The child remains unreaped while live, preventing PID reuse here.
        signals::signal_group(pid, signals::ProcSignal::Hangup)
            .map_err(|error| anyhow::anyhow!("Could not signal session process group: {error}"))
    }

    pub(super) fn on_remove(&self, session: String) -> Result<Outcome> {
        let mut s = relock(&self.state);
        ensure!(
            !s.sessions
                .iter()
                .any(|r| r.id == session && r.lifecycle.live()),
            "Stop session before removing its record"
        );
        s.sessions.retain(|r| r.id != session);
        s.agents.retain(|a| a.session_id != session);
        s.notifications.retain(|n| n.session_id != session);
        s.terminal_notices.retain(|n| n.session_id != session);
        s.revision = s.revision.saturating_add(1);
        drop(s);
        self.history_clear(Some(session), true)?;
        Ok(Outcome::Changed)
    }

    pub(super) fn on_history(&self, session: &str) -> Result<Outcome> {
        let record = relock(&self.state)
            .sessions
            .iter()
            .find(|s| s.id == session)
            .cloned()
            .context("Unknown session")?;
        let text = if let Ok(rt) = self.runtime(session) {
            storage::screen_text(relock(&rt).parser.screen(), 10_000)
        } else {
            self.flush_history()?;
            storage::text(&self.paths, &record)?
        };
        Ok(Outcome::Reply(Response::Text(text)))
    }

    pub(super) fn on_clear_history(&self, session: Option<String>) -> Result<Outcome> {
        self.history_clear(session.clone(), false)?;
        let mut s = relock(&self.state);
        for r in &mut s.sessions {
            if session.as_ref().is_none_or(|id| id == &r.id) {
                r.truncated = true;
            }
        }
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    /// Waits up to 3 s for the history worker to write pending scrollback.
    pub(super) fn flush_history(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.history.send(HistoryJob::Flush(tx))?;
        rx.recv_timeout(Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
}
