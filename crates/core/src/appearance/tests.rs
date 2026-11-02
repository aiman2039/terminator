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
fn islands_dark_is_default_and_older_presets_are_preserved() {
    let default = AppearanceConfig::default();
    assert_eq!(default, AppearanceConfig::islands_dark());
    default.validate().unwrap();
    // `JetBrains` Islands Dark: editor `#191A1C`, tool windows `#212326`.
    assert_eq!(default.window, "#191A1C");
    assert_eq!(default.surface, "#212326");
    assert_eq!(default.selection, "#2A4371");
    assert_eq!(default.git_modified, "#70AEFF");
    assert_eq!(default.git_ignored, "#D69A6B");
    for older in [
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
