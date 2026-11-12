use super::*;

const GHOSTTY: &str = "# Nord-ish\nbackground = 2e3440\nforeground = #d8dee9\n\
    cursor-color = #88c0d0\nselection-background = #434c5e\npalette = 1=#bf616a\n\
    palette = 2=#a3be8c\npalette = 3=#ebcb8b\npalette = 4=#81a1c1\n";

const KITTY: &str = "# kitty theme\nforeground #c0caf5\nbackground #1a1b26\n\
    selection_background #33467c\ncolor1 #f7768e\ncolor2 #9ece6a\ncolor4 #7aa2f7\n";

const ITERM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Background Color</key><dict>
<key>Blue Component</key><real>0.0</real><key>Green Component</key><real>0.0</real>
<key>Red Component</key><real>0.0</real></dict>
<key>Foreground Color</key><dict>
<key>Blue Component</key><real>1</real><key>Green Component</key><real>1</real>
<key>Red Component</key><real>1</real></dict>
<key>Ansi 1 Color</key><dict>
<key>Blue Component</key><real>0</real><key>Green Component</key><real>0</real>
<key>Red Component</key><real>1</real></dict>
</dict></plist>"#;

#[test]
fn ghostty_theme_maps_to_the_color_tokens() {
    let colors = TerminalColors::parse(GHOSTTY).expect("ghostty");
    let next = AppearanceConfig::default().with_terminal_colors(&colors);
    assert_eq!(next.terminal_background, "#2E3440");
    assert_eq!(next.window, "#2E3440");
    assert_eq!(next.text, "#D8DEE9");
    assert_eq!(next.accent, "#88C0D0");
    assert_eq!(next.selection, "#434C5E");
    assert_eq!(next.git_deleted, "#BF616A");
    assert_eq!(next.status_running, "#A3BE8C");
    assert_eq!(next.primary, None);
    next.validate().expect("valid colors");
}

#[test]
fn kitty_and_iterm_themes_import_and_keep_other_settings() {
    let base = AppearanceConfig {
        ui_scale: 1.25,
        ..AppearanceConfig::default()
    };
    let kitty = base.with_terminal_colors(&TerminalColors::parse(KITTY).expect("kitty"));
    assert_eq!(kitty.terminal_foreground, "#C0CAF5");
    assert_eq!(kitty.git_modified, "#7AA2F7");
    assert!((kitty.ui_scale - 1.25).abs() < f32::EPSILON);
    let iterm = base.with_terminal_colors(&TerminalColors::parse(ITERM).expect("iterm"));
    assert_eq!(iterm.window, "#000000");
    assert_eq!(iterm.text, "#FFFFFF");
    assert_eq!(iterm.status_failed, "#FF0000");
    iterm.validate().expect("valid colors");
}

#[test]
fn a_file_without_background_and_foreground_is_refused() {
    assert!(TerminalColors::parse("font-size = 12\n").is_err());
    assert!(TerminalColors::parse("<plist><dict></dict></plist>").is_err());
}

#[test]
fn follow_os_takes_the_light_or_dark_preset_colors_only() {
    let mut config = AppearanceConfig::carbon();
    config.ui_scale = 1.1;
    config.follow = ThemeFollow {
        follow_os_theme: true,
        light_theme: None,
        dark_theme: Some("Islands dark".into()),
    };
    let light = config.for_os(Some(false));
    assert!(light.colors_match(&AppearanceConfig::intellij_light()));
    assert!((light.ui_scale - 1.1).abs() < f32::EPSILON);
    assert!(
        config
            .for_os(Some(true))
            .colors_match(&AppearanceConfig::islands_dark())
    );
    // Unknown OS appearance, or the setting off, keeps the saved colors.
    assert!(config.for_os(None).colors_match(&config));
    config.follow.follow_os_theme = false;
    assert!(config.for_os(Some(false)).colors_match(&config));
}

#[test]
fn saving_follow_keys_keeps_comments_and_removes_unset_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let source = "# mine\n[other]\nkeep = 1\n\n[appearance]\naccent = \"#EF625A\" # accent note\n";
    std::fs::write(&path, source).expect("write");
    let loaded = AppearanceFile::load(&path).expect("load");
    let mut config = loaded.config.clone();
    config.follow.follow_os_theme = true;
    config.follow.dark_theme = Some("Carbon".into());
    let saved = AppearanceFile::save(&path, &config, &loaded.source).expect("save");
    assert!(saved.source.contains("# mine"));
    assert!(saved.source.contains("keep = 1"));
    assert!(saved.source.contains("# accent note"));
    assert!(saved.source.contains("follow_os_theme = true"));
    assert!(!saved.source.contains("light_theme"));
    assert_eq!(AppearanceFile::load(&path).expect("reload").config, config);
    config.follow = ThemeFollow::default();
    let cleared = AppearanceFile::save(&path, &config, &saved.source).expect("save");
    assert!(!cleared.source.contains("follow_os_theme"));
    assert!(!cleared.source.contains("dark_theme"));
}
