//! Native editor text settings, saved as flat keys in the `[appearance]`
//! table of `config.toml` beside the theme colors.
use serde::{Deserialize, Serialize};

/// Font size range, in points.
pub const FONT_SIZE: std::ops::RangeInclusive<f32> = 8.0..=32.0;
/// Line spacing range, as a multiple of the font's row height.
pub const LINE_SPACING: std::ops::RangeInclusive<f32> = 1.0..=2.0;
/// Autosave delay range, in milliseconds.
pub const AUTOSAVE_DELAY_MS: std::ops::RangeInclusive<u32> = 200..=60_000;

/// When the native editor saves on its own. A buffer whose file changed
/// on disk is never autosaved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autosave {
    #[default]
    Off,
    /// When the editor loses keyboard focus.
    FocusLoss,
    /// After `editor_autosave_delay_ms` without typing.
    Delay,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    pub editor_font_size: f32,
    pub editor_line_spacing: f32,
    pub editor_caret_blink: bool,
    /// Color brackets by nesting depth.
    pub editor_rainbow_brackets: bool,
    /// `JetBrains` keymap. In every pane: double Shift, Shift+Cmd+A and
    /// Shift+Cmd+O. In a focused editor Cmd+B, Cmd+E, Cmd+L, F2, Shift+F6
    /// and Cmd+[ / ] also win over app keys.
    pub editor_jetbrains_keys: bool,
    /// Show each line's worst problem after its text ("error lens").
    pub editor_error_lens: bool,
    pub editor_autosave: Autosave,
    pub editor_autosave_delay_ms: u32,
    /// Quick Documentation when the pointer rests on a symbol. Problems
    /// at the pointer show either way.
    pub editor_hover_docs: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            editor_font_size: 13.0,
            editor_line_spacing: 1.2,
            editor_caret_blink: true,
            editor_rainbow_brackets: true,
            editor_jetbrains_keys: true,
            editor_error_lens: false,
            editor_autosave: Autosave::Off,
            editor_autosave_delay_ms: 1000,
            editor_hover_docs: true,
        }
    }
}

impl EditorPrefs {
    /// Values inside their ranges; a hand-edited file out of range
    /// (or NaN) falls back to the default for that key.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let defaults = Self::default();
        let clamp = |value: f32, range: &std::ops::RangeInclusive<f32>, default: f32| {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                default
            }
        };
        Self {
            editor_font_size: clamp(self.editor_font_size, &FONT_SIZE, defaults.editor_font_size),
            editor_line_spacing: clamp(
                self.editor_line_spacing,
                &LINE_SPACING,
                defaults.editor_line_spacing,
            ),
            editor_caret_blink: self.editor_caret_blink,
            editor_rainbow_brackets: self.editor_rainbow_brackets,
            editor_jetbrains_keys: self.editor_jetbrains_keys,
            editor_error_lens: self.editor_error_lens,
            editor_autosave: self.editor_autosave,
            editor_autosave_delay_ms: self
                .editor_autosave_delay_ms
                .clamp(*AUTOSAVE_DELAY_MS.start(), *AUTOSAVE_DELAY_MS.end()),
            editor_hover_docs: self.editor_hover_docs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::{AppearanceConfig, AppearanceFile};

    #[test]
    fn editor_prefs_round_trip_as_flat_appearance_keys_and_keep_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let source = "# mine\n[appearance]\neditor_font_size = 15 # bigger\nother = 1\n";
        std::fs::write(&path, source).unwrap();
        let mut file = AppearanceFile::load(&path).unwrap();
        assert_eq!(file.config.editor.editor_font_size, 15.0);
        assert_eq!(file.config.editor.editor_line_spacing, 1.2);
        file.config.editor.editor_line_spacing = 1.5;
        let saved = AppearanceFile::save(&path, &file.config, &file.source).unwrap();
        assert!(saved.source.contains("# bigger"));
        assert!(saved.source.contains("other = 1"));
        assert!(saved.source.contains("editor_line_spacing = 1.5"));
        let again = AppearanceFile::load(&path).unwrap();
        assert_eq!(again.config.editor.editor_line_spacing, 1.5);
        assert_eq!(AppearanceConfig::default().editor, EditorPrefs::default());
    }

    #[test]
    fn autosave_and_error_lens_round_trip_as_appearance_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[appearance]\n").unwrap();
        let mut file = AppearanceFile::load(&path).unwrap();
        assert_eq!(file.config.editor.editor_autosave, Autosave::Off);
        file.config.editor.editor_autosave = Autosave::FocusLoss;
        file.config.editor.editor_error_lens = true;
        let saved = AppearanceFile::save(&path, &file.config, &file.source).unwrap();
        assert!(saved.source.contains("editor_autosave = \"focus_loss\""));
        let again = AppearanceFile::load(&path).unwrap();
        assert_eq!(again.config.editor.editor_autosave, Autosave::FocusLoss);
        assert!(again.config.editor.editor_error_lens);
    }

    #[test]
    fn out_of_range_editor_prefs_clamp() {
        let prefs = EditorPrefs {
            editor_font_size: 99.0,
            editor_line_spacing: f32::NAN,
            editor_caret_blink: false,
            editor_rainbow_brackets: false,
            editor_jetbrains_keys: false,
            editor_error_lens: true,
            editor_autosave: Autosave::Delay,
            editor_autosave_delay_ms: 5,
            editor_hover_docs: false,
        }
        .sanitized();
        assert_eq!(prefs.editor_autosave_delay_ms, 200);
        assert!(prefs.editor_error_lens);
        assert_eq!(prefs.editor_font_size, 32.0);
        assert_eq!(prefs.editor_line_spacing, 1.2);
        assert!(!prefs.editor_caret_blink);
        assert!(!prefs.editor_rainbow_brackets);
        assert!(!prefs.editor_hover_docs);
    }

    #[test]
    fn hover_docs_are_on_by_default_and_round_trip_as_an_appearance_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Older files have no key: the docs stay on.
        std::fs::write(&path, "[appearance]\neditor_error_lens = true\n").unwrap();
        let mut file = AppearanceFile::load(&path).unwrap();
        assert!(file.config.editor.editor_hover_docs);
        file.config.editor.editor_hover_docs = false;
        let saved = AppearanceFile::save(&path, &file.config, &file.source).unwrap();
        assert!(saved.source.contains("editor_hover_docs = false"));
        assert!(saved.source.contains("editor_error_lens = true"));
        let again = AppearanceFile::load(&path).unwrap();
        assert!(!again.config.editor.editor_hover_docs);
    }
}
