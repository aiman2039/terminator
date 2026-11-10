//! Resume conversation: an explicit click runs the stored resume command.
use crate::*;

impl App {
    /// The stored resume command of an agent in `session`, if any.
    pub(crate) fn resume_command(&self, session: &str) -> Option<String> {
        self.state
            .agents
            .iter()
            .filter(|agent| agent.session_id == session)
            .find_map(|agent| agent.resume.as_ref().map(Resume::display))
    }

    /// Opens a new terminal tab in the ended session's project and folder
    /// and runs `command` there. Only call it for an explicit user click.
    pub(crate) fn resume_conversation(&mut self, session: &Session, command: String) {
        self.hide_center_overlay();
        if self.missing_projects.contains(&session.project_id) {
            return;
        }
        let post = PostCreate {
            input: command,
            enter: true,
        };
        let (project, cwd) = (session.project_id.clone(), Some(session.cwd.clone()));
        self.create_terminal_tab((project, cwd), (None, Some(post)));
    }

    /// The resume command the pane caption's Resume chip types into a live
    /// shell whose agent has stopped (see [`resume_chip_command`]).
    pub(crate) fn resume_chip_command(&self, sid: &str) -> Option<ResumeChipCommand> {
        let session = self.state.sessions.iter().find(|s| s.id == sid)?;
        let shell = session.lifecycle.live() && session.kind == SessionKind::Shell;
        if !shell || !self.backends.contains_key(sid) {
            return None;
        }
        resume_chip_command(&self.present_session(sid))
    }

    /// Types the resume command into the same shell, like Ask agent's
    /// Paste and send. Enter follows only when verified presence found no
    /// live agent. Only call it for an explicit user click.
    pub(crate) fn resume_in_shell(&mut self, sid: &str) {
        let Some(chip) = self.resume_chip_command(sid) else {
            return;
        };
        self.handle_clipboard_paste(sid, &chip.command);
        if chip.confirmed
            && let Some(backend) = self.backends.get_mut(sid)
        {
            backend.process_command(egui_term::BackendCommand::Write(b"\r".to_vec()));
        }
        self.go_session(sid);
    }
}

/// What the pane caption's Resume chip types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResumeChipCommand {
    pub command: String,
    /// Verified presence found no live agent: the click also presses
    /// Enter. Otherwise the chip shows `?` and the user presses Enter.
    pub confirmed: bool,
}

impl ResumeChipCommand {
    /// The chip's tooltip.
    pub(crate) fn tip(&self) -> String {
        if self.confirmed {
            format!("Resume conversation in this shell:\n{}", self.command)
        } else {
            format!(
                "Cannot confirm the agent stopped. Press Enter to run.\n{}",
                self.command
            )
        }
    }
}

/// A live shell earns the Resume chip when its latest hook state is
/// Stopped or Failed and that agent left a resume command. Verified
/// presence with a live agent hides the chip. Without verified presence
/// (an older daemon, for example) the chip types the command without
/// Enter, so the command never runs inside a running agent by itself.
pub(crate) fn resume_chip_command(
    presented: &super::AgentPresentation,
) -> Option<ResumeChipCommand> {
    if presented.verified && presented.live {
        return None;
    }
    let hook = presented.diagnostics.as_ref()?.hook.as_ref()?;
    if !matches!(hook.state, AgentState::Stopped | AgentState::Failed) {
        return None;
    }
    let command = hook.resume.as_ref().map(Resume::display)?;
    Some(ResumeChipCommand {
        command,
        confirmed: presented.verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::agents::{PresenceOutcome, TerminalPresence};

    fn state(hook: AgentState, resume: bool, agents: &[&str]) -> State {
        let mut state: State = serde_json::from_value(serde_json::json!({
            "capabilities": [terminator_core::AGENT_PRESENCE_CAPABILITY],
            "sessions": [{
                "id": "s", "project_id": "p", "label": "zsh", "cwd": "/",
                "kind": "shell", "file": null, "lifecycle": "running", "created": 0,
                "exit_code": null, "rows": 24, "cols": 80, "generation": "g",
                "pid": 1, "truncated": false, "cwd_confirmed": true
            }],
            "agents": [{
                "invocation_id": "i", "session_id": "s", "kind": "claude",
                "provider_session_id": "abc", "state": hook, "sequence": null,
                "updated": 1, "resume": null
            }]
        }))
        .expect("state");
        if resume {
            state.agents[0].resume = Some(Resume {
                program: "claude".into(),
                args: vec!["--resume".into(), "abc".into()],
            });
        }
        state.presence.push(TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: agents
                .iter()
                .map(|kind| terminator_core::agents::DetectedAgent {
                    kind: (*kind).into(),
                    process: terminator_core::agents::ProcessIdentity {
                        pid: 7,
                        start_time: 1,
                    },
                    foreground: true,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: 100,
        });
        state
    }

    fn chip(state: &State) -> Option<ResumeChipCommand> {
        let mut cache = super::super::cache::PresentationCache::default();
        let presented = cache.get(super::super::GetInput {
            state,
            id: "s",
            now: 100,
            fresh: Some(true),
        });
        resume_chip_command(&presented)
    }

    #[test]
    fn verified_presence_without_a_live_agent_runs_the_resume_command_with_enter() {
        let stopped = chip(&state(AgentState::Stopped, true, &[])).expect("chip");
        assert!(stopped.command.contains("abc"));
        assert!(stopped.confirmed, "a click also presses Enter");
        assert!(
            stopped
                .tip()
                .starts_with("Resume conversation in this shell:")
        );
        let failed = chip(&state(AgentState::Failed, true, &[])).expect("chip");
        assert!(failed.confirmed);
    }

    #[test]
    fn resume_chip_hides_while_running_without_command_or_with_a_verified_live_agent() {
        assert_eq!(chip(&state(AgentState::Running, true, &[])), None);
        assert_eq!(chip(&state(AgentState::Completed, true, &[])), None);
        assert_eq!(chip(&state(AgentState::Stopped, false, &[])), None);
        assert_eq!(chip(&state(AgentState::Stopped, true, &["claude"])), None);
    }

    #[test]
    fn unverified_presence_types_the_resume_command_without_enter() {
        // An older daemon: no presence capability, no observation.
        let mut state = state(AgentState::Stopped, true, &[]);
        state.presence.clear();
        state.capabilities.clear();
        let unsure = chip(&state).expect("chip");
        assert!(unsure.command.contains("abc"));
        assert!(!unsure.confirmed, "the user presses Enter");
        assert!(
            unsure
                .tip()
                .starts_with("Cannot confirm the agent stopped. Press Enter to run.")
        );
        // Still no chip while the hook says the agent runs.
        let mut running = state.clone();
        running.agents[0].state = AgentState::Running;
        assert_eq!(chip(&running), None);
    }
}
