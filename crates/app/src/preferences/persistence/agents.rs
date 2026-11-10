use serde::{Deserialize, Serialize};
use terminator_core::AgentState;

/// All-live agent state filter, set from the status-bar Working and Done
/// counts or the filter menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStateFilter {
    #[default]
    All,
    Working,
    Done,
}

impl AgentStateFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Working, Self::Done];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Any state",
            Self::Working => "Working",
            Self::Done => "Done",
        }
    }

    /// A live session with this hook state shows under the filter. The
    /// states match the status-bar counts (Running, Completed).
    pub fn matches(self, state: Option<AgentState>) -> bool {
        match self {
            Self::All => true,
            Self::Working => state == Some(AgentState::Running),
            Self::Done => state == Some(AgentState::Completed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_filter_keeps_only_the_counted_state() {
        let states = [
            None,
            Some(AgentState::Running),
            Some(AgentState::Completed),
            Some(AgentState::WaitingInput),
        ];
        let kept = |filter: AgentStateFilter| {
            states
                .iter()
                .filter(|state| filter.matches(**state))
                .count()
        };
        assert_eq!(kept(AgentStateFilter::All), 4);
        assert!(AgentStateFilter::Working.matches(Some(AgentState::Running)));
        assert_eq!(kept(AgentStateFilter::Working), 1);
        assert!(AgentStateFilter::Done.matches(Some(AgentState::Completed)));
        assert_eq!(kept(AgentStateFilter::Done), 1);
    }

    #[test]
    fn missing_state_filter_preference_means_all() {
        #[derive(Deserialize)]
        struct Prefs {
            #[serde(default)]
            filter: AgentStateFilter,
        }
        let prefs: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.filter, AgentStateFilter::All);
        let prefs: Prefs = serde_json::from_str(r#"{"filter":"done"}"#).unwrap();
        assert_eq!(prefs.filter, AgentStateFilter::Done);
    }
}
