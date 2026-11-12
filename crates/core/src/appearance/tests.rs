use super::*;
#[test]
fn defaults_comments_unrelated_keys_and_conflict() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    assert_eq!(
        AppearanceFile::load(&p).unwrap().config,
        AppearanceConfig::default()
    );
    assert!(!p.exists());
    let source = "# user comment\n[other]\nkey = 42\n[appearance]\ntext = '#FFFFFF' # keep this\n";
    fs::write(&p, source).unwrap();
    let mut file = AppearanceFile::load(&p).unwrap();
    file.config.text = "#123456".into();
    let saved = AppearanceFile::save(&p, &file.config, &file.source).unwrap();
    assert!(saved.source.contains("# keep this"));
    assert!(saved.source.contains("key = 42"));
    assert!(AppearanceFile::save(&p, &file.config, source).is_err());
}
#[cfg(unix)]
#[test]
fn saving_symlinked_config_keeps_link_and_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("dotfiles.toml");
    let link = d.path().join("config.toml");
    fs::write(&target, "[other]\nkey = 1\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut file = AppearanceFile::load(&link).unwrap();
    file.config.text = "#123456".into();
    AppearanceFile::save(&link, &file.config, &file.source).unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::read_to_string(&target).unwrap().contains("#123456"));
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
}
#[test]
fn invalid_external_values_are_rejected() {
    for color in ["red", "#💥abc", "#GG0000"] {
        let c = AppearanceConfig {
            text: color.into(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }
    assert!(
        AppearanceConfig {
            border_width: f32::NAN,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn carbon_is_default_and_older_presets_are_preserved() {
    let default = AppearanceConfig::default();
    assert_eq!(default, AppearanceConfig::carbon());
    default.validate().unwrap();
    assert_eq!(default.window, "#111113");
    assert_eq!(default.surface, "#1A191B");
    assert_eq!(default.selection, "#343136");
    assert_eq!(default.accent, "#EF625A");
    for older in [
        AppearanceConfig::islands_dark(),
        AppearanceConfig::intellij_dark(),
        AppearanceConfig::terminator_dark(),
    ] {
        older.validate().unwrap();
        assert!(!default.colors_match(&older));
    }
    assert_eq!(AppearanceConfig::intellij_dark().window, "#1E1F22");
    assert_eq!(AppearanceConfig::terminator_dark().window, "#26282C");
}
#[test]
fn light_preset_validates_and_reads_as_light() {
    let light = AppearanceConfig::intellij_light();
    light.validate().unwrap();
    assert!(light.is_light());
    for dark in [
        AppearanceConfig::islands_dark(),
        AppearanceConfig::intellij_dark(),
        AppearanceConfig::terminator_dark(),
        AppearanceConfig::high_contrast(),
    ] {
        assert!(!dark.is_light());
    }
}
#[test]
fn high_contrast_and_density_round_trip() {
    AppearanceConfig::high_contrast().validate().unwrap();
    let mut compact = AppearanceConfig {
        density: Density::Compact,
        ..Default::default()
    };
    assert!(compact.compact());
    assert_eq!(compact.row_height(), 24.0);
    compact.apply_accent("#3871E1".into());
    assert_eq!(compact.accent, "#3871E1");
    assert!(compact.selection.starts_with('#'));
}
#[test]
fn config_without_font_keys_keeps_bundled_font_and_writes_no_key() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    fs::write(&p, "[appearance]\ntext = '#FFFFFF'\n").unwrap();
    let file = AppearanceFile::load(&p).unwrap();
    assert_eq!(file.config.fonts, FontPrefs::default());
    assert_eq!(file.config.fonts.terminal(), None);
    let saved = AppearanceFile::save(&p, &file.config, &file.source).unwrap();
    assert!(!saved.source.contains("terminal_font"));
    assert!(!saved.source.contains("cjk_font"));
}
#[test]
fn saved_theme_without_primary_keeps_the_accent_and_unset_removes_the_key() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    fs::write(&p, "[appearance]\naccent = '#EF625A'\nother = 1 # keep\n").unwrap();
    let file = AppearanceFile::load(&p).unwrap();
    assert_eq!(file.config.primary, None, "a saved theme stays as it was");
    assert_eq!(file.config.primary_color(), "#EF625A");
    let mut chosen = file.config.clone();
    chosen.primary = Some("#EEEDEB".into());
    let saved = AppearanceFile::save(&p, &chosen, &file.source).unwrap();
    assert!(saved.source.contains("primary = \"#EEEDEB\""));
    let mut cleared = chosen.clone();
    cleared.primary = None;
    let saved = AppearanceFile::save(&p, &cleared, &saved.source).unwrap();
    assert!(!saved.source.contains("primary"));
    assert!(saved.source.contains("other = 1 # keep"));
    let mut bad = cleared;
    bad.primary = Some("red".into());
    assert!(bad.validate().is_err());
    assert_eq!(
        AppearanceConfig::carbon().primary.as_deref(),
        Some("#EEEDEB")
    );
    assert_eq!(AppearanceConfig::intellij_light().primary, None);
}
#[test]
fn font_choice_round_trips_and_clearing_removes_the_key() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    fs::write(&p, "[appearance]\nother = 1 # keep\n").unwrap();
    let mut file = AppearanceFile::load(&p).unwrap();
    file.config.fonts.terminal_font = Some("Hack Nerd Font Mono".into());
    file.config.fonts.cjk_font = Some("PingFang SC".into());
    let saved = AppearanceFile::save(&p, &file.config, &file.source).unwrap();
    assert!(
        saved
            .source
            .contains("terminal_font = \"Hack Nerd Font Mono\"")
    );
    assert!(saved.source.contains("# keep"));
    let again = AppearanceFile::load(&p).unwrap();
    assert_eq!(again.config.fonts.terminal(), Some("Hack Nerd Font Mono"));
    assert_eq!(again.config.fonts.cjk(), Some("PingFang SC"));
    let mut cleared = again.config.clone();
    cleared.fonts = FontPrefs::default();
    let saved = AppearanceFile::save(&p, &cleared, &again.source).unwrap();
    assert!(!saved.source.contains("terminal_font"));
    assert!(!saved.source.contains("cjk_font"));
    assert!(saved.source.contains("other = 1"));
}
#[test]
fn blank_font_name_means_bundled() {
    let prefs = FontPrefs {
        terminal_font: Some("  ".into()),
        cjk_font: None,
    };
    assert_eq!(prefs.terminal(), None);
}

/// WCAG 2.x relative luminance of a `#RRGGBB` color.
fn luminance(hex: &str) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = rgb(hex).unwrap();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn presets() -> [(&'static str, AppearanceConfig); 6] {
    [
        ("carbon", AppearanceConfig::carbon()),
        ("islands dark", AppearanceConfig::islands_dark()),
        ("intellij dark", AppearanceConfig::intellij_dark()),
        ("terminator dark", AppearanceConfig::terminator_dark()),
        ("high contrast", AppearanceConfig::high_contrast()),
        ("intellij light", AppearanceConfig::intellij_light()),
    ]
}

#[test]
fn presets_meet_contrast_floors() {
    for (name, theme) in presets() {
        let texts = [
            ("text", &theme.text),
            ("secondary", &theme.secondary),
            ("git_added", &theme.git_added),
            ("git_modified", &theme.git_modified),
            ("git_deleted", &theme.git_deleted),
            ("git_untracked", &theme.git_untracked),
            ("git_ignored", &theme.git_ignored),
            ("status_running", &theme.status_running),
            ("status_waiting", &theme.status_waiting),
            ("status_failed", &theme.status_failed),
        ];
        for (token, color) in texts {
            for (fill, floor) in [
                (&theme.surface, 4.5),
                (&theme.window, 4.5),
                (&theme.selection, 3.0),
            ] {
                let ratio = contrast(color, fill);
                assert!(
                    ratio >= floor,
                    "{name}: {token} {color} on {fill} is {ratio:.2}:1, below {floor}:1"
                );
            }
        }
    }
}

#[test]
fn the_accent_picker_keeps_selected_text_readable() {
    for (name, mut theme) in presets() {
        let before = theme.selection.clone();
        theme.apply_accent("#3574F0".into());
        assert!(
            contrast(&theme.text, &theme.selection) >= 7.0,
            "{name}: text on {} is too weak",
            theme.selection
        );
        if name == "carbon" {
            assert_eq!(theme.selection, before, "a neutral selection stays");
        }
    }
    let mut light = AppearanceConfig::intellij_light();
    light.apply_accent("#3574F0".into());
    assert_eq!(light.selection, "#DBE6FC");
}
