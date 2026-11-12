//! Built-in preset table and "Follow OS light/dark": one preset for a
//! light OS appearance and one for a dark one.
use super::AppearanceConfig;
use serde::{Deserialize, Serialize};

/// Built-in presets by Settings name.
pub type Preset = (&'static str, fn() -> AppearanceConfig);

/// Every built-in preset, in Settings order.
pub const PRESETS: [Preset; 6] = [
    ("Carbon", AppearanceConfig::carbon),
    ("Islands dark", AppearanceConfig::islands_dark),
    ("IntelliJ dark", AppearanceConfig::intellij_dark),
    ("High contrast", AppearanceConfig::high_contrast),
    ("Terminator dark", AppearanceConfig::terminator_dark),
    ("IntelliJ light", AppearanceConfig::intellij_light),
];

/// Preset used for a light OS appearance when none is chosen.
pub const DEFAULT_LIGHT_THEME: &str = "IntelliJ light";
/// Preset used for a dark OS appearance when none is chosen.
pub const DEFAULT_DARK_THEME: &str = "Carbon";

/// A built-in preset by name.
#[must_use]
pub fn preset(name: &str) -> Option<AppearanceConfig> {
    PRESETS
        .iter()
        .find(|(label, _)| *label == name)
        .map(|(_, make)| make())
}

/// `config.toml` keys for following the OS appearance. Unset keys are
/// never written, and saving removes them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFollow {
    /// Take colors from `light_theme` or `dark_theme` by the OS setting.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub follow_os_theme: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
}

impl ThemeFollow {
    /// Optional keys; saving removes each one that is unset.
    pub const KEYS: [&'static str; 3] = ["follow_os_theme", "light_theme", "dark_theme"];

    /// Preset name for the OS appearance.
    #[must_use]
    pub fn preset_name(&self, dark: bool) -> &str {
        if dark {
            self.dark_theme.as_deref().unwrap_or(DEFAULT_DARK_THEME)
        } else {
            self.light_theme.as_deref().unwrap_or(DEFAULT_LIGHT_THEME)
        }
    }
}

impl AppearanceConfig {
    /// The configuration to paint with. While `follow_os_theme` is on and
    /// the OS says light or dark (`os_dark`), the colors come from that
    /// preset; every other key stays. Unknown OS appearance or an unknown
    /// preset name keeps the saved colors.
    #[must_use]
    pub fn for_os(&self, os_dark: Option<bool>) -> Self {
        let mut next = self.clone();
        let (true, Some(dark)) = (self.follow.follow_os_theme, os_dark) else {
            return next;
        };
        if let Some(source) = preset(self.follow.preset_name(dark)) {
            next.copy_colors(&source);
        }
        next
    }

    /// Take every color token, `primary` included, from `source`.
    pub fn copy_colors(&mut self, source: &Self) {
        let mut source = source.clone();
        for ((_, value), (_, from)) in self.colors_mut().into_iter().zip(source.colors_mut()) {
            value.clone_from(from);
        }
        self.primary.clone_from(&source.primary);
    }
}
