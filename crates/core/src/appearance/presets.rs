//! Built-in color presets.
use super::{AppearanceConfig, Density};

impl AppearanceConfig {
    /// Carbon: warm charcoal, neutral selections, and a restrained red accent.
    /// Explicit saved colors remain unchanged when the default changes.
    #[must_use]
    pub fn carbon() -> Self {
        Self {
            window: "#111113".into(),
            surface: "#1A191B".into(),
            hover: "#29282B".into(),
            border: "#302E32".into(),
            text: "#EEEDEB".into(),
            secondary: "#A5A1A8".into(),
            accent: "#EF625A".into(),
            // A neutral main action keeps red for destructive actions.
            primary: Some("#EEEDEB".into()),
            selection: "#343136".into(),
            terminal_background: "#111113".into(),
            terminal_foreground: "#EEEDEB".into(),
            git_added: "#91C788".into(),
            git_modified: "#DDB878".into(),
            git_deleted: "#EF827A".into(),
            git_untracked: "#91C788".into(),
            git_ignored: "#969099".into(),
            status_running: "#91C788".into(),
            status_waiting: "#DDB878".into(),
            status_failed: "#EF827A".into(),
            ..Self::intellij_dark()
        }
    }
    /// `JetBrains` "Islands Dark" palette (IntelliJ 2025.2+, from
    /// `ManyIslandsDark.theme.json` and `IslandSchemeDark.xml`): near-black
    /// islands (editor and tool windows), a slightly lighter frame, Islands file-status
    /// colors. Saved user themes load their explicit colors
    /// and are never rewritten to this.
    #[must_use]
    pub fn islands_dark() -> Self {
        Self {
            window: "#191A1C".into(),
            surface: "#212326".into(),
            hover: "#2B2D30".into(),
            border: "#2B2D30".into(),
            text: "#D1D3D9".into(),
            secondary: "#9FA2A8".into(),
            accent: "#3871E1".into(),
            selection: "#2A4371".into(),
            terminal_background: "#191A1C".into(),
            terminal_foreground: "#BCBEC4".into(),
            git_added: "#73BD79".into(),
            git_modified: "#70AEFF".into(),
            git_deleted: "#E57571".into(),
            git_untracked: "#E88F89".into(),
            git_ignored: "#D69A6B".into(),
            status_running: "#5FAD65".into(),
            status_waiting: "#D6AE58".into(),
            status_failed: "#E37774".into(),
            ..Self::intellij_dark()
        }
    }
    /// `JetBrains` New UI dark palette (before Islands): charcoal window,
    /// lighter frame surface, muted blue selection.
    #[must_use]
    pub fn intellij_dark() -> Self {
        Self {
            version: 1,
            window: "#1E1F22".into(),
            surface: "#2B2D30".into(),
            hover: "#393B40".into(),
            border: "#393B40".into(),
            text: "#DFE1E5".into(),
            secondary: "#9DA1A8".into(),
            accent: "#3574F0".into(),
            primary: None,
            selection: "#214283".into(),
            terminal_background: "#1E1F22".into(),
            terminal_foreground: "#DFE1E5".into(),
            git_added: "#81B88B".into(),
            git_modified: "#E2C08D".into(),
            git_deleted: "#EB7A68".into(),
            git_untracked: "#73C991".into(),
            git_ignored: "#9A9DA3".into(),
            status_running: "#79BAA3".into(),
            status_waiting: "#E2C08D".into(),
            status_failed: "#EB7A68".into(),
            border_width: 1.0,
            pane_divider_width: 1.0,
            ui_scale: 1.0,
            density: Density::Comfortable,
            editor: crate::editor_prefs::EditorPrefs::default(),
            fonts: super::FontPrefs::default(),
            follow: super::ThemeFollow::default(),
        }
    }
    /// Pre-IDE palette, kept as a selectable preset.
    #[must_use]
    pub fn terminator_dark() -> Self {
        Self {
            version: 1,
            window: "#26282C".into(),
            surface: "#191A1C".into(),
            hover: "#212326".into(),
            border: "#33353B".into(),
            text: "#D1D3D9".into(),
            secondary: "#9FA2A8".into(),
            accent: "#3871E1".into(),
            selection: "#233558".into(),
            terminal_background: "#191A1C".into(),
            terminal_foreground: "#D1D3D9".into(),
            git_added: "#81B88B".into(),
            git_modified: "#E2C08D".into(),
            git_deleted: "#EB7A68".into(),
            git_untracked: "#73C991".into(),
            git_ignored: "#9A9DA3".into(),
            status_running: "#79BAA3".into(),
            status_waiting: "#E2C08D".into(),
            status_failed: "#EB7A68".into(),
            ..Self::intellij_dark()
        }
    }
    #[must_use]
    pub fn high_contrast() -> Self {
        Self {
            window: "#0A0A0C".into(),
            surface: "#000000".into(),
            hover: "#1A1A1E".into(),
            border: "#8C8C96".into(),
            text: "#FFFFFF".into(),
            secondary: "#C8C8D0".into(),
            accent: "#5B9DFF".into(),
            primary: None,
            selection: "#1A3358".into(),
            terminal_background: "#000000".into(),
            terminal_foreground: "#FFFFFF".into(),
            git_added: "#9BE09B".into(),
            git_modified: "#F0D48A".into(),
            git_deleted: "#FF6B5A".into(),
            git_untracked: "#9BE09B".into(),
            git_ignored: "#808080".into(),
            status_running: "#8EE0C0".into(),
            status_waiting: "#F0D48A".into(),
            status_failed: "#FF6B5A".into(),
            ..Default::default()
        }
    }
    /// `JetBrains` New UI light palette.
    #[must_use]
    pub fn intellij_light() -> Self {
        Self {
            window: "#F7F8FA".into(),
            surface: "#FFFFFF".into(),
            hover: "#EBECF0".into(),
            border: "#D3D5DB".into(),
            text: "#1E1F22".into(),
            secondary: "#6C707E".into(),
            accent: "#3574F0".into(),
            primary: None,
            selection: "#D4E2FF".into(),
            terminal_background: "#FFFFFF".into(),
            terminal_foreground: "#1E1F22".into(),
            git_added: "#2F7D3B".into(),
            git_modified: "#9A6700".into(),
            git_deleted: "#C7222D".into(),
            git_untracked: "#237A4E".into(),
            git_ignored: "#6E7178".into(),
            status_running: "#1F7F69".into(),
            status_waiting: "#9A6700".into(),
            status_failed: "#C7222D".into(),
            ..Default::default()
        }
    }
}
