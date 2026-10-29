//! Host theme colors for the editor surface: current line, gutter,
//! find matches, status bar and the vim mode badge.
use crate::view::ModeColors;
use egui::Color32;

/// Editor surface colors. Hosts pass theme tokens; without them
/// [`EditorPalette::from_visuals`] derives quiet defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorPalette {
    /// Full-width fill under the caret's row.
    pub current_line: Color32,
    /// Gutter band behind line numbers, fold arrows and blame.
    pub gutter_fill: Color32,
    /// 1px hairline between the gutter and the text.
    pub gutter_rule: Color32,
    pub line_number: Color32,
    /// The caret line's number, brighter than the others.
    pub current_line_number: Color32,
    /// Find-bar matches, and the selected one.
    pub find_match: Color32,
    pub find_current: Color32,
    /// Status bar band and its top hairline.
    pub status_fill: Color32,
    pub status_rule: Color32,
    pub status_text: Color32,
    pub modes: ModeColors,
}

impl EditorPalette {
    #[must_use]
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        let text = visuals.text_color();
        let rule = visuals.widgets.noninteractive.bg_stroke.color;
        let accent = visuals.selection.stroke.color;
        Self {
            current_line: text.gamma_multiply(0.04),
            gutter_fill: text.gamma_multiply(0.02),
            gutter_rule: rule,
            line_number: visuals.weak_text_color(),
            current_line_number: text,
            find_match: accent.gamma_multiply(0.3),
            find_current: accent.gamma_multiply(0.6),
            status_fill: visuals.panel_fill,
            status_rule: rule,
            status_text: visuals.weak_text_color(),
            modes: ModeColors::from_visuals(visuals),
        }
    }
}

impl<B: crate::doc::Buffer> super::SourceView<'_, B> {
    /// The host palette, or one derived from `visuals`.
    pub(super) fn palette(&self, visuals: &egui::Visuals) -> EditorPalette {
        self.options
            .palette
            .unwrap_or_else(|| EditorPalette::from_visuals(visuals))
    }
}
