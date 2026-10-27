use super::*;
use terminator_core::idle_close::{Outcome, Status};

/// `ConPTY` has no foreground process group: treat any live child of the shell
/// as owning the terminal instead.
#[cfg(not(unix))]
fn shell_has_live_children(pid: u32) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system.processes().values().any(|process| {
        process
            .parent()
            .is_some_and(|parent| parent.as_u32() == pid)
    })
}

/// Return a session signalled for idle close to service when its exit was
/// not confirmed. Only the `Stopping` mark set by idle close is undone.
fn reopen_unconfirmed(lifecycle: &mut Lifecycle) {
    if *lifecycle == Lifecycle::Stopping {
        *lifecycle = Lifecycle::Running;
    }
}

impl Shared {
    fn idle_preflight(&self, session: &str) -> Result<Option<u32>> {
        let state = relock(&self.state);
        let record = state
            .sessions
            .iter()
            .find(|r| r.id == session)
            .context("Unknown session")?;
        ensure!(
            record.remote.is_none(),
            "Remote SSH activity cannot be verified locally; close the session explicitly"
        );
        if !record.lifecycle.live() {
            return Ok(None);
        }
        ensure!(
            agents::select_session_agent(&state.agents, session).is_none_or(|agent| {
                matches!(
                    agent.state,
                    AgentState::Stopped | AgentState::Completed | AgentState::Failed
                )
            }),
            "An agent is active or its lifecycle is unknown"
        );
        let runtime = self.runtime(session)?;
        let runtime = relock(&runtime);
        let expected = runtime
            .shell_executable
            .as_deref()
            .context("Launched shell identity is unavailable")?;
        let pid = terminator_core::idle_close::verify_shell(record, expected)?;
        ensure!(
            !runtime.ended && !runtime.closing,
            "Session exit is still being reconciled"
        );
        ensure!(
            runtime.inputs_in_flight == 0,
            "Terminal input is still being forwarded"
        );
        #[cfg(unix)]
        {
            let Ok(shell_pid) = i32::try_from(pid) else {
                anyhow::bail!("A foreground command owns the terminal");
            };
            ensure!(
                runtime.master.process_group_leader() == Some(shell_pid),
                "A foreground command owns the terminal"
            );
        }
        #[cfg(not(unix))]
        ensure!(
            !shell_has_live_children(pid),
            "A foreground command owns the terminal"
        );
        Ok(Some(pid))
    }

    fn stale_generation_outcomes(sessions: Vec<String>) -> Vec<Outcome> {
        sessions
            .into_iter()
            .map(|session| Outcome {
                session,
                status: Status::StaleGeneration,
                reason: "Daemon generation changed".into(),
            })
            .collect()
    }
    fn preflight_failed_outcomes(
        sessions: Vec<String>,
        checks: Vec<Result<Option<u32>>>,
    ) -> Vec<Outcome> {
        sessions
            .into_iter()
            .zip(checks)
            .map(|(session, result)| {
                let (status, reason) = match result {
                    Ok(None) => (Status::AlreadyEnded, "Session already ended".into()),
                    Ok(Some(_)) => (
                        Status::Busy,
                        "Another target failed preflight; no sessions were signalled".into(),
                    ),
                    Err(error) => (Status::Unknown, error.to_string()),
                };
                Outcome { session, status, reason }
            })
            .collect()
    }
    fn signal_one(&self, session: &str) -> (Status, String) {
        match self.idle_preflight(session) {
            Ok(None) => (Status::AlreadyEnded, "Session already ended".into()),
            Ok(Some(pid)) => {
                if terminator_core::signals::signal_group(
                    pid,
                    terminator_core::signals::ProcSignal::Hangup,
                )
                .is_ok()
                {
                    if let Ok(runtime) = self.runtime(session) {
                        relock(&runtime).closing = true;
                    }
                    let mut state = relock(&self.state);
                    if let Some(record) = state
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == session && s.lifecycle.live())
                    {
                        record.lifecycle = Lifecycle::Stopping;
                    }
                    state.revision = state.revision.saturating_add(1);
                    (Status::Closed, "Idle shell signalled; waiting for exit".into())
                } else {
                    (Status::Failed, std::io::Error::last_os_error().to_string())
                }
            }
            Err(error) => (Status::Failed, error.to_string()),
        }
    }
    fn confirm_outcomes(&self, outcomes: &mut [Outcome]) {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(2))
            .unwrap_or_else(Instant::now);
        for outcome in outcomes.iter_mut() {
            if outcome.status != Status::Closed {
                continue;
            }
            while relock(&self.state)
                .sessions
                .iter()
                .any(|s| s.id == outcome.session && s.lifecycle.live())
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            if relock(&self.state)
                .sessions
                .iter()
                .any(|s| s.id == outcome.session && s.lifecycle.live())
            {
                if let Ok(runtime) = self.runtime(&outcome.session) {
                    relock(&runtime).closing = false;
                }
                let mut state = relock(&self.state);
                if let Some(record) = state.sessions.iter_mut().find(|s| s.id == outcome.session) {
                    reopen_unconfirmed(&mut record.lifecycle);
                }
                state.revision = state.revision.saturating_add(1);
                drop(state);
                outcome.status = Status::Failed;
                outcome.reason =
                    "Signal delivered, but exit was not confirmed; view preserved".into();
            } else {
                outcome.reason = "Idle shell exited".into();
            }
        }
    }

    pub(super) fn close_idle(
        &self,
        generation: String,
        mut sessions: Vec<String>,
    ) -> Result<Response> {
        ensure!(
            !sessions.is_empty() && sessions.len() <= 256,
            "Expected 1–256 target sessions"
        );
        sessions.sort();
        sessions.dedup();
        let mut outcomes = Vec::new();
        {
            let _operation = relock(&self.terminal_operations);
            if relock(&self.state).generation != generation {
                return Ok(Response::IdleSessionsClosed(Self::stale_generation_outcomes(sessions)));
            }
            let checks: Vec<_> = sessions.iter().map(|s| self.idle_preflight(s)).collect();
            if checks.iter().any(Result::is_err) {
                return Ok(Response::IdleSessionsClosed(Self::preflight_failed_outcomes(sessions, checks)));
            }
            let mut partial_failure = false;
            for session in sessions {
                if partial_failure {
                    outcomes.push(Outcome { session, status: Status::Failed, reason: "Not signalled after another target failed; no rollback of previous signals".into() });
                    continue;
                }
                let (status, reason) = self.signal_one(&session);
                let failed = status == Status::Failed;
                outcomes.push(Outcome { session, status, reason });
                partial_failure = failed;
            }
        }
        self.confirm_outcomes(&mut outcomes);
        Ok(Response::IdleSessionsClosed(outcomes))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfirmed_idle_close_returns_the_session_to_service() {
        let mut lifecycle = Lifecycle::Stopping;
        reopen_unconfirmed(&mut lifecycle);
        assert_eq!(lifecycle, Lifecycle::Running);
        for ended in [Lifecycle::Ended, Lifecycle::Interrupted] {
            let mut lifecycle = ended.clone();
            reopen_unconfirmed(&mut lifecycle);
            assert_eq!(lifecycle, ended);
        }
    }
}
