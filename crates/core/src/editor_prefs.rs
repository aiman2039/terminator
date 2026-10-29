//! Native editor text settings, saved as flat keys in the `[appearance]`
//! table of `config.toml` beside the theme colors.
use serde::{Deserialize, Serialize};

/// Font size range, in points.
pub const FONT_SIZE: std::ops::RangeInclusive<f32> = 8.0..=32.0;
/// Line spacing range, as a multiple of the font's row height.
pub const LINE_SPACING: std::ops::RangeInclusive<f32> = 1.0..=2.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    pub editor_font_size: f32,
    pub editor_line_spacing: f32,
    pub editor_caret_blink: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            editor_font_size: 13.0,
            editor_line_spacing: 1.2,
            editor_caret_blink: true,
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
    fn out_of_range_editor_prefs_clamp() {
        let prefs = EditorPrefs {
            editor_font_size: 99.0,
            editor_line_spacing: f32::NAN,
            editor_caret_blink: false,
        }
        .sanitized();
        assert_eq!(prefs.editor_font_size, 32.0);
        assert_eq!(prefs.editor_line_spacing, 1.2);
        assert!(!prefs.editor_caret_blink);
    }
}
