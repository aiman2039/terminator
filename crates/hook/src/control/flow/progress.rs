//! Where one running step is, read from the daemon snapshot only: hook
//! reports and the session's lifecycle, never terminal text.
use super::super::agents::{Mark, WaitFor, WaitStep, latest_mark, wait_step};
use terminator_core::*;

/// What the runner waits for in a running step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// A command step: waits for its agent (see [`progress`]).
    Command(Wait),
    /// A gate: waits for the user's answer in the run folder.
    Gate,
}

/// What a running command step waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Wait {
    /// The command was sent. The prompt waits for the agent's first hook
    /// report, which shows that the agent started.
    Starting { prompt: String },
    /// The agent works. `since` is its newest hook record when the work
    /// started; only a newer record can end the step.
    Working { since: Option<Mark> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Progress {
    Pending,
    /// The agent started: send the prompt now.
    SendPrompt,
    Done,
    Failed(&'static str),
}

/// The daemon removes an ended shell with its exit code, so an ended
/// terminal cannot show that its step was good: only a hook report can.
const ENDED: &str = "The terminal ended before the agent reported that it was done";

/// One poll of a command step.
pub(super) fn progress(state: &State, session: &str, wait: &Wait) -> Progress {
    let Some(found) = state.sessions.iter().find(|s| s.id == session) else {
        return Progress::Failed(ENDED);
    };
    match wait {
        Wait::Starting { .. } if !found.lifecycle.live() => Progress::Failed(ENDED),
        Wait::Starting { .. } if latest_mark(state, session).is_some() => Progress::SendPrompt,
        Wait::Starting { .. } => Progress::Pending,
        Wait::Working { since } => {
            match wait_step(state, session, WaitFor::AnyStop, Some(since)) {
                Ok(WaitStep::Reached(AgentState::Completed | AgentState::WaitingInput)) => {
                    Progress::Done
                }
                Ok(WaitStep::Reached(AgentState::Failed)) => Progress::Failed("The agent failed"),
                Ok(WaitStep::Reached(AgentState::Stopped)) => Progress::Failed("The agent stopped"),
                // A permission question is the user's to answer in the terminal.
                Ok(WaitStep::Reached(_) | WaitStep::Pending) => Progress::Pending,
                Ok(WaitStep::Ended) | Err(_) => Progress::Failed(ENDED),
            }
        }
    }
}

/// Most characters of one result note in a prompt.
const MAX_RESULT: usize = 2000;

/// The prompt with the result notes of the needed steps after it. A note
/// comes only from a step's own `flow report done --note`, never from
/// terminal text.
pub(super) fn with_results<'a>(
    prompt: &str,
    results: impl Iterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut text = prompt.to_owned();
    for (index, (step, note)) in results.enumerate() {
        text.push_str(if index == 0 {
            "\nResults of the steps before this one:"
        } else {
            ";"
        });
        // A note is an agent's text and it is typed into a terminal: a
        // control character in it (Escape, Ctrl-C) must not be a key.
        let short: String = note
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_RESULT)
            .collect();
        text.push_str(&format!(" [{step}] {short}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(lifecycle: Lifecycle, exit_code: Option<u32>, agent: Option<AgentState>) -> State {
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
            exit_code,
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
            kind: "claude".into(),
            provider_session_id: None,
            state,
            sequence: None,
            updated: 100,
            resume: None,
            process: None,
        }));
        state
    }

    fn starting() -> Wait {
        Wait::Starting { prompt: "p".into() }
    }

    #[test]
    fn prompt_waits_for_the_first_hook_report() {
        let quiet = state(Lifecycle::Running, None, None);
        assert_eq!(progress(&quiet, "s", &starting()), Progress::Pending);
        let started = state(Lifecycle::Running, None, Some(AgentState::Unknown));
        assert_eq!(progress(&started, "s", &starting()), Progress::SendPrompt);
    }

    #[test]
    fn step_fails_when_the_terminal_ends_before_the_agent_starts() {
        let ended = state(Lifecycle::Ended, Some(0), None);
        assert!(matches!(
            progress(&ended, "s", &starting()),
            Progress::Failed(_)
        ));
        assert!(matches!(
            progress(&ended, "other", &starting()),
            Progress::Failed(_)
        ));
    }

    #[test]
    fn old_hook_record_does_not_end_the_step() {
        let done = state(Lifecycle::Running, None, Some(AgentState::Completed));
        let since = latest_mark(&done, "s");
        let working = Wait::Working { since };
        assert_eq!(progress(&done, "s", &working), Progress::Pending);
        let mut next = done.clone();
        next.agents[0].updated = 105;
        assert_eq!(progress(&next, "s", &working), Progress::Done);
    }

    #[test]
    fn step_ends_by_the_new_hook_state() {
        let working = Wait::Working { since: None };
        let case = |agent| progress(&state(Lifecycle::Running, None, Some(agent)), "s", &working);
        assert_eq!(case(AgentState::WaitingInput), Progress::Done);
        assert_eq!(case(AgentState::Completed), Progress::Done);
        assert_eq!(case(AgentState::Running), Progress::Pending);
        assert_eq!(case(AgentState::WaitingPermission), Progress::Pending);
        assert!(matches!(case(AgentState::Failed), Progress::Failed(_)));
        assert!(matches!(case(AgentState::Stopped), Progress::Failed(_)));
    }

    #[test]
    fn ended_terminal_never_ends_a_step_as_done() {
        let working = Wait::Working { since: None };
        for exit_code in [Some(0), Some(1), None] {
            let ended = state(Lifecycle::Ended, exit_code, None);
            assert_eq!(progress(&ended, "s", &working), Progress::Failed(ENDED));
        }
        // The daemon removes an ended shell from the snapshot.
        let gone = state(Lifecycle::Running, None, None);
        assert_eq!(progress(&gone, "other", &working), Progress::Failed(ENDED));
    }

    #[test]
    fn prompt_gets_the_result_notes_of_the_needed_steps() {
        assert_eq!(with_results("Build it.", std::iter::empty()), "Build it.");
        let results = [("plan", "Use plan A.\nSee PLAN.md"), ("api", "v2")];
        let text = with_results("Build it.", results.into_iter());
        assert_eq!(
            one_line(&text),
            "Build it. Results of the steps before this one: [plan] Use plan A. See PLAN.md; [api] v2"
        );
        let long = "x".repeat(MAX_RESULT + 50);
        let text = with_results("p", [("a", long.as_str())].into_iter());
        assert_eq!(text.matches('x').count(), MAX_RESULT);
        let keys = with_results("p", [("a", "ok\u{1b}[201~\u{3}rm -rf x")].into_iter());
        assert!(
            !keys.chars().any(|c| c.is_control() && c != '\n'),
            "{keys:?}"
        );
    }

    #[test]
    fn prompt_is_typed_as_one_line() {
        assert_eq!(
            one_line("Write the plan.\n  Keep it short.\r\n"),
            "Write the plan. Keep it short."
        );
    }
}
