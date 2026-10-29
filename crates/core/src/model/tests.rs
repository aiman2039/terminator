use super::*;

#[test]
fn detached_vnc_timeout_defaults_to_30_minutes_and_is_bounded() {
    let mut s = Settings::default();
    assert_eq!(s.vnc_detached_timeout_minutes, 30);
    s.vnc_detached_timeout_minutes = 0;
    assert!(s.validate().is_ok(), "0 keeps detached sessions");
    s.vnc_detached_timeout_minutes = 1441;
    assert!(s.validate().is_err());
    let old: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(
        old.vnc_detached_timeout_minutes, 30,
        "older saved settings get the default"
    );
}

#[test]
fn new_settings_defaults_include_editor_close_timeout() {
    let s = Settings::default();
    assert_eq!(s.editor_close_timeout_secs, 1);
}

#[test]
fn new_settings_defaults_include_diff_split_default() {
    let s = Settings::default();
    assert!(!s.diff_split_default);
}

#[test]
fn settings_serialization_round_trips_new_fields() {
    let s = Settings {
        editor_close_timeout_secs: 5,
        diff_split_default: true,
        ..Settings::default()
    };
    let json = serde_json::to_string(&s).unwrap();
    let restored: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.editor_close_timeout_secs, 5);
    assert!(restored.diff_split_default);
}
#[test]
fn ntfy_settings_default_off_validate_and_round_trip() {
    let mut settings: Settings = serde_json::from_str("{}").unwrap();
    assert!(!settings.ntfy_enabled);
    settings.ntfy_enabled = true;
    assert!(settings.validate().is_err());
    settings.ntfy_channel = "phone_123-test".into();
    settings.ntfy_machine = "My laptop".into();
    settings.validate().unwrap();
    let restored: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored, settings);
    for channel in ["https://ntfy.sh/topic", "a/b", "a?b", "a\nB", "has space"] {
        settings.ntfy_channel = channel.into();
        assert!(settings.validate().is_err());
    }
    settings.ntfy_channel = "valid".into();
    settings.ntfy_machine = "bad\nheader".into();
    assert!(settings.validate().is_err());
}

#[test]
fn disabled_ntfy_accepts_any_channel_and_machine_contents() {
    // Turning the toggle off must always save, even with leftover or
    // cleared field contents. Re-enabling still requires valid values.
    let mut settings = Settings::default();
    settings.validate().unwrap();
    settings.ntfy_channel = "has space!".into();
    settings.ntfy_machine = "bad\nheader".into();
    settings.validate().unwrap();
    settings.ntfy_channel.clear();
    settings.ntfy_machine.clear();
    settings.validate().unwrap();
    settings.ntfy_enabled = true;
    assert!(settings.validate().is_err());
    settings.ntfy_channel = "ok-channel_1".into();
    settings.ntfy_machine = "My laptop".into();
    settings.validate().unwrap();
}

#[test]
fn automatic_update_checks_default_on_and_round_trip() {
    assert!(Settings::default().automatic_update_checks);
    let parsed: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(parsed.automatic_update_checks);
    let off: Settings =
        serde_json::from_value(serde_json::json!({"automatic_update_checks": false})).unwrap();
    assert!(!off.automatic_update_checks);
    let encoded = serde_json::to_value(&off).unwrap();
    assert_eq!(encoded["automatic_update_checks"], false);
}

#[test]
fn notification_sound_defaults_on_and_round_trips() {
    assert!(Settings::default().notification_sound);
    let parsed: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(parsed.notification_sound);
    let off: Settings =
        serde_json::from_value(serde_json::json!({"notification_sound": false})).unwrap();
    assert!(!off.notification_sound);
    let encoded = serde_json::to_value(&off).unwrap();
    assert_eq!(encoded["notification_sound"], false);
}
