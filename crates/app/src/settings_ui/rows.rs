//! Single Settings rows that Find Action lists. Picking one opens its
//! section with the row label as the search, so that row shows alone.
//! A test paints each section and checks every label here is a real row.
use super::SettingsSection as S;

/// `(section, row label)`; the label is the row's `field_visible` label.
pub(crate) const SETTINGS_ROWS: &[(S, &str)] = &[
    (S::Appearance, "Theme"),
    (S::Appearance, "Follow OS appearance"),
    (S::Appearance, "Import terminal theme"),
    (S::Appearance, "Accent"),
    (S::Appearance, "Density"),
    (S::Appearance, "Interface size"),
    (S::Appearance, "Terminal font"),
    (S::Appearance, "CJK fallback font"),
    (S::Terminal, "Shell override"),
    (S::Terminal, "Font size"),
    (S::Terminal, "Type-ahead"),
    (S::Editor, "Editor mode"),
    (S::Editor, "External editor"),
    (S::Editor, "Vim keybindings"),
    (S::Editor, "Inlay hints"),
    (S::Editor, "Autosave"),
    (S::Keymap, "Keymap"),
    (S::Languages, "Spell checking"),
    (S::Languages, "User-defined language servers"),
    (S::Git, "Diff viewer"),
    (S::Git, "Pull requests"),
    (S::Notifications, "Play sound with desktop notifications"),
    (S::Notifications, "Dismiss notifications"),
    (S::Remote, "Detached VNC sessions"),
    (S::History, "Local History days"),
];

/// The palette title of one row: `Settings › Section › Row`.
pub(crate) fn row_title(section: S, label: &str) -> String {
    format!("Settings › {} › {label}", section.title())
}
