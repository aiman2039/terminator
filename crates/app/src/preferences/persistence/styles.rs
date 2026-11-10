use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalStyle {
    pub color: Option<String>,
    pub icon: Option<String>,
    pub pinned: bool,
    /// The user named this terminal's tab: its label wins over the
    /// program's OSC window title.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub label_manual: bool,
}

/// GUI-only project display overrides: name, color, icon, and whether the
/// project is pinned to the top of the list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectStyle {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub pinned: bool,
}
