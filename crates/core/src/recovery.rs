//! The inventory approved by the user, carried unchanged to detached recovery.
use crate::{State, *};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestartInventory {
    pub generation: String,
    pub sessions: BTreeSet<String>,
}

impl RestartInventory {
    pub fn capture(state: &State) -> Self {
        Self {
            generation: state.generation.clone(),
            sessions: state
                .sessions
                .iter()
                .filter(|s| s.lifecycle.live())
                .map(|s| s.id.clone())
                .collect(),
        }
    }

    pub fn validate(&self, state: &State) -> Result<()> {
        ensure!(
            self.generation == state.generation,
            "Service changed after confirmation; no sessions were stopped. Review the current service and retry"
        );
        ensure!(
            state
                .sessions
                .iter()
                .filter(|s| s.lifecycle.live())
                .all(|s| self.sessions.contains(&s.id)),
            "Session inventory changed after confirmation; no sessions were stopped. Review the current sessions and retry"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        let session: Session = serde_json::from_value(serde_json::json!({
            "id":"approved","project_id":"p","label":"shell","cwd":"/tmp",
            "kind":"shell","lifecycle":"running","created":0,"rows":24,"cols":80,
            "generation":"older-owner","truncated":false,"cwd_confirmed":false
        }))
        .unwrap();
        State {
            generation: "active-owner".into(),
            sessions: vec![session],
            ..Default::default()
        }
    }
    #[test]
    fn confirmation_rejects_replacement_sessions_even_with_the_same_count() {
        let mut state = state();
        let inventory = RestartInventory::capture(&state);
        state.sessions[0].id = "created-after-click".into();
        assert!(inventory.validate(&state).is_err());
        assert_eq!(inventory.sessions, ["approved".into()].into());
    }
    #[test]
    fn ended_approved_sessions_are_allowed_but_generation_changes_are_not() {
        let mut state = state();
        let inventory = RestartInventory::capture(&state);
        state.sessions[0].lifecycle = Lifecycle::Ended;
        inventory.validate(&state).unwrap();
        state.generation = "replacement-owner".into();
        assert!(inventory.validate(&state).is_err());
    }
}
