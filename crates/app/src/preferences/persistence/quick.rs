use serde::{Deserialize, Serialize};

/// The quick terminal: the project its scratch terminal opens in (`None`
/// is the selected project) and that terminal's session once it exists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickTerminalPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}
