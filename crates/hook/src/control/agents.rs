//! `ctl agents` lists agent state; `ctl wait` blocks until a state is reached.
//! Both read the daemon snapshot only: no new request, no terminal text.
use super::{WaitArgs, lookup::Ctl};
use anyhow::Result;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use terminator_core::*;

/// `ctl wait` exit code when the timeout passes first.
pub(super) const WAIT_TIMEOUT: i32 = 2;
/// `ctl wait` exit code when the session ends first.
pub(super) const WAIT_ENDED: i32 = 3;
const POLL: Duration = Duration::from_millis(500);

/// The state groups `ctl wait --state` accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum WaitFor {
    /// Waiting for input or permission.
    Waiting,
    Done,
    Failed,
    /// Waiting, done, failed or stopped.
    AnyStop,
}

impl WaitFor {
    pub(super) fn matches(self, state: AgentState) -> bool {
        match self {
            Self::Waiting => matches!(
                state,
                AgentState::WaitingInput | AgentState::WaitingPermission
            ),
            Self::Done => state == AgentState::Completed,
            Self::Failed => state == AgentState::Failed,
            Self::AnyStop => !matches!(state, AgentState::Running | AgentState::Unknown),
        }
    }
}

/// The newest agent record of a session.
fn latest_agent<'a>(state: &'a State, session: &str) -> Option<&'a Agent> {
    state
        .agents
        .iter()
        .filter(|agent| agent.session_id == session)
        .max_by_key(|agent| (agent.updated, agent.sequence))
}

/// Identity of an agent record version: updated time, sequence and record id.
pub(super) type Mark = (u64, Option<u64>, String);

fn mark(agent: &Agent) -> Mark {
    (agent.updated, agent.sequence, agent.invocation_id.clone())
}

/// The mark of the newest agent record of a session, if any.
pub(super) fn latest_mark(state: &State, session: &str) -> Option<Mark> {
    latest_agent(state, session).map(mark)
}

/// Agent rows: session, project, label, kind, state, and seconds since the
/// last hook report.
pub(super) fn agent_rows(state: &State, project: Option<&str>, now: u64) -> Vec<Value> {
    state
        .sessions
        .iter()
        .filter(|session| project.is_none_or(|id| session.project_id == id))
        .filter_map(|session| {
            let agent = latest_agent(state, &session.id)?;
            Some(json!({
                "session": session.id,
                "project": session.project_id,
                "label": session.label,
                "live": session.lifecycle.live(),
                "kind": agent.kind,
                "state": agent.state,
                "age": now.saturating_sub(agent.updated),
            }))
        })
        .collect()
}

/// One poll result of `ctl wait`.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum WaitStep {
    Reached(AgentState),
    Ended,
    Pending,
}

/// `since` is the mark recorded at the start of `ctl wait --changed`
/// (`Some(None)` when the session had no agent record then); a record with
/// the same mark does not count.
pub(super) fn wait_step(
    state: &State,
    session: &str,
    want: WaitFor,
    since: Option<&Option<Mark>>,
) -> Result<WaitStep> {
    let Some(found) = state.sessions.iter().find(|s| s.id == session) else {
        anyhow::bail!("Unknown session ID");
    };
    let agent = latest_agent(state, session)
        .filter(|agent| since.is_none_or(|old| old.as_ref() != Some(&mark(agent))))
        .map(|agent| agent.state);
    if let Some(reached) = agent.filter(|state| want.matches(*state)) {
        return Ok(WaitStep::Reached(reached));
    }
    Ok(if found.lifecycle.live() {
        WaitStep::Pending
    } else {
        WaitStep::Ended
    })
}

impl Ctl {
    pub(super) fn agents(&self, project: Option<String>) -> Result<Value> {
        let project = project
            .map(|name| Self::project(&self.state, &name))
            .transpose()?;
        let rows = agent_rows(&self.state, project.as_ref().map(|p| p.id.as_str()), now());
        Ok(Value::Array(rows))
    }

    /// Polls the snapshot every 500 ms. Returns the reached state, or the
    /// process exit code for a timeout or an ended session.
    pub(super) fn wait(&self, args: WaitArgs) -> Result<std::result::Result<Value, i32>> {
        self.capability(agents::CAPABILITY)?;
        let deadline = args
            .timeout
            .and_then(|s| Instant::now().checked_add(Duration::from_secs(s)));
        let mut state = self.state.clone();
        let since = args.changed.then(|| latest_mark(&state, &args.session));
        loop {
            match wait_step(&state, &args.session, args.state, since.as_ref())? {
                WaitStep::Reached(reached) => {
                    return Ok(Ok(json!({"session": args.session, "state": reached})));
                }
                WaitStep::Ended => return Ok(Err(WAIT_ENDED)),
                WaitStep::Pending => {}
            }
            if deadline.is_some_and(|at| Instant::now() >= at) {
                return Ok(Err(WAIT_TIMEOUT));
            }
            std::thread::sleep(POLL);
            state = Self::snapshot(&self.paths)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(lifecycle: Lifecycle, agent: Option<AgentState>) -> State {
        let mut state = State::default();
        state.sessions.push(Session {
            remote: None,
            review: false,
            id: "s".into(),
            project_id: "p".into(),
            label: "agent".into(),
            cwd: "/p".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        });
        state.agents.extend(agent.map(|state| Agent {
            invocation_id: "i".into(),
            session_id: "s".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state,
            sequence: None,
            updated: 100,
            resume: None,
            process: None,
        }));
        state
    }

    #[test]
    fn ctl_wait_exits_on_waiting_state() {
        let waiting = state(Lifecycle::Running, Some(AgentState::WaitingPermission));
        assert_eq!(
            wait_step(&waiting, "s", WaitFor::Waiting, None).unwrap(),
            WaitStep::Reached(AgentState::WaitingPermission)
        );
        let running = state(Lifecycle::Running, Some(AgentState::Running));
        assert_eq!(
            wait_step(&running, "s", WaitFor::AnyStop, None).unwrap(),
            WaitStep::Pending
        );
    }

    #[test]
    fn ctl_wait_changed_ignores_the_previous_turn_state() {
        let done = state(Lifecycle::Running, Some(AgentState::Completed));
        let since = Some(latest_mark(&done, "s"));
        assert_eq!(
            wait_step(&done, "s", WaitFor::Done, since.as_ref()).unwrap(),
            WaitStep::Pending
        );
        let mut next = done.clone();
        next.agents[0].updated = 105;
        assert_eq!(
            wait_step(&next, "s", WaitFor::Done, since.as_ref()).unwrap(),
            WaitStep::Reached(AgentState::Completed)
        );
        next.agents[0].state = AgentState::Running;
        assert_eq!(
            wait_step(&next, "s", WaitFor::Done, since.as_ref()).unwrap(),
            WaitStep::Pending
        );
    }

    #[test]
    fn ctl_wait_changed_accepts_a_first_agent_record() {
        let empty = state(Lifecycle::Running, None);
        let since = Some(latest_mark(&empty, "s"));
        let waiting = state(Lifecycle::Running, Some(AgentState::WaitingInput));
        assert_eq!(
            wait_step(&waiting, "s", WaitFor::Waiting, since.as_ref()).unwrap(),
            WaitStep::Reached(AgentState::WaitingInput)
        );
    }

    #[test]
    fn ctl_wait_reports_an_ended_session() {
        let ended = state(Lifecycle::Ended, Some(AgentState::Running));
        assert_eq!(
            wait_step(&ended, "s", WaitFor::Done, None).unwrap(),
            WaitStep::Ended
        );
        assert!(wait_step(&ended, "other", WaitFor::Done, None).is_err());
    }

    #[test]
    fn ctl_agents_lists_state_and_age() {
        let rows = agent_rows(
            &state(Lifecycle::Running, Some(AgentState::Completed)),
            None,
            130,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["state"], "completed");
        assert_eq!(rows[0]["age"], 30);
        assert!(agent_rows(&state(Lifecycle::Running, None), None, 0).is_empty());
        assert!(
            agent_rows(
                &state(Lifecycle::Running, Some(AgentState::Running)),
                Some("x"),
                0
            )
            .is_empty()
        );
    }
}
