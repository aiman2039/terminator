use serde::{Deserialize, Serialize};

/// Local echo prediction for shell terminals (#83). GUI-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeAhead {
    /// Predict once the measured echo time is above 100 ms.
    #[default]
    Auto,
    On,
    Off,
}

impl TypeAhead {
    pub const fn terminal_mode(self) -> egui_term::TypeAheadMode {
        match self {
            Self::Auto => egui_term::TypeAheadMode::Auto,
            Self::On => egui_term::TypeAheadMode::On,
            Self::Off => egui_term::TypeAheadMode::Off,
        }
    }
}
