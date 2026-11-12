//! Usage chips of a session's agent: the model and permission mode the
//! agent reported in its hooks (`State::agent_usage`). Never read from
//! terminal text; older daemons send none, so no chip shows.
use crate::App;
use terminator_core::State;

/// Longest model name on a chip.
const MAX_MODEL_CHARS: usize = 24;

/// Chip texts for the most recently updated agent of `session`.
pub(crate) fn usage_chips(state: &State, session: &str) -> Vec<String> {
    if state.agent_usage.is_empty() {
        return Vec::new();
    }
    let Some(usage) = state
        .agents
        .iter()
        .filter(|agent| agent.session_id == session)
        .max_by_key(|agent| agent.updated)
        .and_then(|agent| state.agent_usage.get(&agent.invocation_id))
    else {
        return Vec::new();
    };
    let mut chips = Vec::new();
    if let Some(model) = &usage.model {
        let mut short: String = model.chars().take(MAX_MODEL_CHARS).collect();
        if model.chars().count() > MAX_MODEL_CHARS {
            short.push('…');
        }
        chips.push(short);
    }
    if let Some(mode) = usage.mode_label() {
        chips.push(mode.to_owned());
    }
    chips
}

impl App {
    pub(crate) fn agent_usage_chips(&self, session: &str) -> Vec<String> {
        usage_chips(&self.state, session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::{Agent, AgentState, agent_workflow::AgentUsage};

    fn agent(id: &str, updated: u64) -> Agent {
        Agent {
            invocation_id: id.into(),
            session_id: "s".into(),
            kind: "claude".into(),
            provider_session_id: None,
            state: AgentState::Running,
            sequence: None,
            updated,
            resume: None,
            process: None,
        }
    }

    #[test]
    fn chips_show_only_what_the_newest_agent_reported() {
        let mut state = State::default();
        assert!(usage_chips(&state, "s").is_empty());
        state.agents = vec![agent("old", 1), agent("new", 2)];
        state.agent_usage.insert(
            "old".into(),
            AgentUsage::new(Some("old-model"), Some("plan")).unwrap(),
        );
        assert!(
            usage_chips(&state, "s").is_empty(),
            "the newest agent reported nothing"
        );
        state.agent_usage.insert(
            "new".into(),
            AgentUsage::new(
                Some("a-very-long-model-name-from-the-hook"),
                Some("acceptEdits"),
            )
            .unwrap(),
        );
        assert_eq!(
            usage_chips(&state, "s"),
            ["a-very-long-model-name-f…", "Accept edits"]
        );
        assert!(usage_chips(&state, "other").is_empty());
    }
}
