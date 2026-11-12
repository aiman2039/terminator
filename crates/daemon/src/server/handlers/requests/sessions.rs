use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use terminator_core::*;

use super::super::super::shared::{HistoryJob, Runtime, Shared, relock};
use super::Outcome;
use crate::{signals, storage};

/// Delay before Stop repeats the hang-up to a session that is still live.
const STOP_REPEAT: Duration = Duration::from_secs(1);

impl Shared {
    pub(super) fn on_prune_history(&self, budget: u64) -> Result<Outcome> {
        let ids = self.history_request(|tx| HistoryJob::Prune(budget, tx))?;
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
        // Nothing changes on a prompt; do not bump the revision or persist.
        Ok(Outcome::Reply(Response::Ok))
    }

    pub(super) fn cwd_unchanged(&self, session: &str, path: &std::path::Path) -> bool {
        cwd_unchanged(&relock(&self.state), session, path)
    }

    pub(super) fn on_session_for_pids(&self, pids: &[u32]) -> Outcome {
        Outcome::Reply(match session_for_pids(&relock(&self.state), pids) {
            Some(id) => Response::Text(id.into()),
            None => Response::Ok,
        })
    }

    pub(super) fn on_screen(&self, session: &str) -> Result<Outcome> {
        let runtime = self.runtime(session)?;
        let screen = relock(&runtime).parser.screen().contents();
        Ok(Outcome::Reply(Response::Text(screen)))
    }

    pub(super) fn on_command_blocks(&self, session: &str, since: u64) -> Result<Outcome> {
        let runtime = self.runtime(session)?;
        // Bounded so the reply fits one IPC frame (#191).
        let blocks = relock(&runtime).commands.tracker().reply_blocks(since);
        Ok(Outcome::Reply(Response::CommandBlocks(blocks)))
    }

    pub(super) fn on_create(
        self: &Arc<Self>,
        input: super::super::sessions::CreateInput,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::Created(self.create(input)?)))
    }

    /// A fresh local shell with saved output above its prompt. Nothing is
    /// typed into it but the project's init command; no agent is launched.
    pub(super) fn on_create_restored(self: &Arc<Self>, request: Request) -> Result<Outcome> {
        let Request::CreateRestored {
            project,
            cwd,
            label,
            seed,
        } = request
        else {
            anyhow::bail!("Expected a restore request");
        };
        ensure!(label.len() <= 256, "Label too long");
        ensure!(
            seed.len() <= agent_workflow::MAX_RESTORE_SEED.saturating_mul(2),
            "Saved output too long"
        );
        let mut session = self.create(super::super::sessions::CreateInput {
            project,
            cwd: Some(cwd),
            file: None,
            line: None,
            column: None,
            launch: crate::Launch::Shell,
            seed: Some(seed),
        })?;
        if !label.trim().is_empty() {
            self.on_rename(&session.id, label.clone())?;
            session.label = label;
        }
        Ok(Outcome::Reply(Response::Created(session)))
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
        ensure!(relock(&rt).child_live(), "Session ended");
        if let Some(pid) = rec.pid {
            Self::hang_up_session(&rt, pid)?;
            // A shell can catch the hang-up just before it `exec`s a command
            // and lose it, which leaves the session stopping forever. Send
            // it once more if the session is still live after a short wait.
            let rt = Arc::clone(&rt);
            std::thread::spawn(move || {
                std::thread::sleep(STOP_REPEAT);
                if relock(&rt).child_live() {
                    let _ = Self::hang_up_session(&rt, pid);
                }
            });
        }
        rec.lifecycle = Lifecycle::Stopping;
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    fn hang_up_session(
        #[cfg_attr(
            not(unix),
            expect(unused_variables, reason = "only Unix signals the foreground job")
        )]
        rt: &Arc<Mutex<Runtime>>,
        pid: u32,
    ) -> Result<()> {
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
            // Copy under the lock; walk scrollback after releasing it (#237).
            let screen = relock(&rt).parser.screen().clone();
            storage::screen_text(screen, 10_000)
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
        self.history_request(HistoryJob::Flush)
    }
}

/// True when `session` is live, local and already at the confirmed `path`.
fn cwd_unchanged(state: &State, session: &str, path: &std::path::Path) -> bool {
    state.sessions.iter().any(|r| {
        r.id == session
            && r.lifecycle.live()
            && r.remote.is_none()
            && r.cwd_confirmed
            && r.cwd == path
    })
}

/// The one live session whose shell PID is among `pids`; `None` when none or
/// several match.
fn session_for_pids<'a>(state: &'a State, pids: &[u32]) -> Option<&'a str> {
    let mut matches = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| pids.contains(&pid)));
    match (matches.next(), matches.next()) {
        (Some(session), None) => Some(session.id.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(sessions: &[(&str, u32)]) -> State {
        let mut state = State::default();
        for (id, pid) in sessions {
            state.sessions.push(Session {
                review: false,
                remote: None,
                id: (*id).into(),
                project_id: String::new(),
                label: String::new(),
                cwd: PathBuf::from("/work"),
                kind: SessionKind::Shell,
                file: None,
                lifecycle: Lifecycle::Running,
                created: 0,
                exit_code: None,
                rows: 24,
                cols: 80,
                generation: String::new(),
                pid: Some(*pid),
                truncated: false,
                cwd_confirmed: true,
            });
        }
        state
    }

    #[test]
    fn repeated_prompt_cwd_is_not_a_change() {
        let state = state_with(&[("a", 10)]);
        assert!(cwd_unchanged(&state, "a", std::path::Path::new("/work")));
        assert!(!cwd_unchanged(&state, "a", std::path::Path::new("/other")));
        assert!(!cwd_unchanged(&state, "b", std::path::Path::new("/work")));
    }

    #[test]
    fn hook_ancestors_map_to_exactly_one_live_session() {
        let state = state_with(&[("a", 10), ("b", 20)]);
        assert_eq!(session_for_pids(&state, &[99, 20, 1]), Some("b"));
        assert_eq!(session_for_pids(&state, &[99]), None);
        assert_eq!(session_for_pids(&state, &[10, 20]), None);
    }
}
