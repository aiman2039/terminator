use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalStyle {
    pub color: Option<String>,
    pub icon: Option<String>,
}

/// GUI-only project display overrides: name, color, and icon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectStyle {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}
