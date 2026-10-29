//! Native editor tokens: surface colors derived from the theme, and the
//! editor text settings. Set by [`super::apply`] like the chrome tints, so
//! every viewport and the Settings preview share one value.
use super::color;
use std::sync::RwLock;
use terminator_core::appearance::AppearanceConfig;
use terminator_core::editor_prefs::EditorPrefs;
use terminator_native_edit::view::{EditorPalette, ModeColors};

/// Editor colors and text settings for the current theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorStyle {
    pub palette: EditorPalette,
    pub prefs: EditorPrefs,
}

static EDITOR: RwLock<Option<EditorStyle>> = RwLock::new(None);

/// The editor style set by the last [`super::apply`].
pub fn editor_style() -> EditorStyle {
    EDITOR
        .read()
        .ok()
        .and_then(|style| *style)
        .unwrap_or_else(|| editor_style_for(&AppearanceConfig::default()))
}

pub(super) fn set_editor_style(theme: &AppearanceConfig) {
    if let Ok(mut slot) = EDITOR.write() {
        *slot = Some(editor_style_for(theme));
    }
}

/// `JetBrains`-style editor tokens from the theme palette: a faint
/// current-line band, a barely tinted gutter with a hairline, dim line
/// numbers with a bright current one, and soft green find matches.
pub fn editor_style_for(theme: &AppearanceConfig) -> EditorStyle {
    let background = color(&theme.window);
    let text = color(&theme.text);
    let secondary = color(&theme.secondary);
    let found = color(&theme.git_added);
    let light = theme.is_light();
    let palette = EditorPalette {
        current_line: background.lerp_to_gamma(text, if light { 0.05 } else { 0.04 }),
        gutter_fill: background.lerp_to_gamma(text, 0.02),
        gutter_rule: background.lerp_to_gamma(color(&theme.border), 0.7),
        line_number: background.lerp_to_gamma(secondary, 0.6),
        current_line_number: secondary.lerp_to_gamma(text, 0.6),
        find_match: background.lerp_to_gamma(found, if light { 0.25 } else { 0.22 }),
        find_current: background.lerp_to_gamma(found, if light { 0.45 } else { 0.42 }),
        status_fill: color(&theme.surface),
        status_rule: color(&theme.border),
        status_text: secondary,
        modes: ModeColors {
            normal: color(&theme.accent),
            insert: found,
            visual: color(&theme.status_waiting),
            command: secondary,
        },
    };
    EditorStyle {
        palette,
        prefs: theme.editor.sanitized(),
    }
}
