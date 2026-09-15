use super::*;
use terminator_core::idle_close::{Outcome, Status};

impl Shared {
    fn idle_preflight(&self, session: &str) -> Result<Option<u32>> {
        let state = self.state.lock().unwrap();
        let record = state
            .sessions
            .iter()
            .find(|r| r.id == session)
            .context("Unknown session")?;
        if !record.lifecycle.live() {
            return Ok(None);
        }
        ensure!(
            !state.agents.iter().any(|a| a.session_id == session
                && !matches!(
                    a.state,
                    AgentState::Stopped | AgentState::Completed | AgentState::Failed
                )),
            "An agent is active or its lifecycle is unknown"
        );
        let runtime = self.runtime(session)?;
        let runtime = runtime.lock().unwrap();
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
        ensure!(
            runtime.prompt.ready,
            "No current authenticated prompt readiness; keep running or explicitly terminate"
        );
        ensure!(
            runtime.master.process_group_leader() == Some(pid as i32),
            "A foreground command owns the terminal"
        );
        Ok(Some(pid))
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
            // All input forwarding, readiness transitions, and close decisions share this lock.
            let _operation = self.terminal_operations.lock().unwrap();
            if self.state.lock().unwrap().generation != generation {
                return Ok(Response::IdleSessionsClosed(
                    sessions
                        .into_iter()
                        .map(|session| Outcome {
                            session,
                            status: Status::StaleGeneration,
                            reason: "Daemon generation changed".into(),
                        })
                        .collect(),
                ));
            }
            let checks: Vec<_> = sessions.iter().map(|s| self.idle_preflight(s)).collect();
            if checks.iter().any(Result::is_err) {
                return Ok(Response::IdleSessionsClosed(
                    sessions
                        .into_iter()
                        .zip(checks)
                        .map(|(session, result)| {
                            let (status, reason) = match result {
                                Ok(None) => (Status::AlreadyEnded, "Session already ended".into()),
                                Ok(Some(_)) => (
                                    Status::Busy,
                                    "Another target failed preflight; no sessions were signalled"
                                        .into(),
                                ),
                                Err(error) => (Status::Unknown, error.to_string()),
                            };
                            Outcome {
                                session,
                                status,
                                reason,
                            }
                        })
                        .collect(),
                ));
            }
            let mut partial_failure = false;
            for session in sessions {
                if partial_failure {
                    outcomes.push(Outcome { session, status: Status::Failed, reason: "Not signalled after another target failed; no rollback of previous signals".into() });
                    continue;
                }
                // Reinspect immediately before each signal; a partial failure cannot roll back signals.
                let result = self.idle_preflight(&session);
                let (status, reason) = match result {
                    Ok(None) => (Status::AlreadyEnded, "Session already ended".into()),
                    Ok(Some(pid)) => {
                        if unsafe { libc::kill(-(pid as i32), libc::SIGHUP) } == 0 {
                            if let Ok(runtime) = self.runtime(&session) {
                                let mut runtime = runtime.lock().unwrap();
                                runtime.prompt.ready = false;
                                runtime.closing = true;
                            }
                            let mut state = self.state.lock().unwrap();
                            if let Some(record) = state
                                .sessions
                                .iter_mut()
                                .find(|s| s.id == session && s.lifecycle.live())
                            {
                                record.lifecycle = Lifecycle::Stopping;
                            }
                            state.revision += 1;
                            (
                                Status::Closed,
                                "Idle shell signalled; waiting for exit".into(),
                            )
                        } else {
                            (Status::Failed, std::io::Error::last_os_error().to_string())
                        }
                    }
                    Err(error) => (Status::Failed, error.to_string()),
                };
                let failed = status == Status::Failed;
                outcomes.push(Outcome {
                    session,
                    status,
                    reason,
                });
                partial_failure = failed;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        for outcome in &mut outcomes {
            if outcome.status != Status::Closed {
                continue;
            }
            while self
                .state
                .lock()
                .unwrap()
                .sessions
                .iter()
                .any(|s| s.id == outcome.session && s.lifecycle.live())
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            if self
                .state
                .lock()
                .unwrap()
                .sessions
                .iter()
                .any(|s| s.id == outcome.session && s.lifecycle.live())
            {
                outcome.status = Status::Failed;
                outcome.reason =
                    "Signal delivered, but exit was not confirmed; view preserved".into();
            } else {
                outcome.reason = "Idle shell exited".into();
            }
        }
        Ok(Response::IdleSessionsClosed(outcomes))
    }
}
