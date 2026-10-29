//! Diff tint tokens: how strongly the Git status colors show in the diff
//! viewer. Every diff surface reads these instead of hard-coded alphas.
use eframe::egui::Color32;

/// Alphas over `git_added` / `git_deleted` / `git_modified`, plus the
/// line-number gutter shade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffTints {
    /// Changed-row background.
    pub row: u8,
    /// Changed word or character background, over the row tint.
    pub word: u8,
    /// Connector band fill between the split panes.
    pub band: u8,
    /// Connector band outline, selected-hunk outline, and action chevrons.
    pub edge: u8,
    /// Line-number gutter shade.
    pub gutter: Color32,
}

/// Dark themes: a soft row tint with clearly stronger changed words.
pub const DIFF_TINTS_DARK: DiffTints = DiffTints {
    row: 25,
    word: 70,
    band: 30,
    edge: 110,
    gutter: Color32::from_black_alpha(40),
};

/// Light themes: the saturated light-theme Git colors need a little more
/// alpha to read on white, and a lighter gutter shade.
pub const DIFF_TINTS_LIGHT: DiffTints = DiffTints {
    row: 32,
    word: 80,
    band: 36,
    edge: 140,
    gutter: Color32::from_black_alpha(10),
};

/// Tint tokens for a dark (`true`) or light theme.
pub fn diff_tints(dark: bool) -> DiffTints {
    if dark {
        DIFF_TINTS_DARK
    } else {
        DIFF_TINTS_LIGHT
    }
}
