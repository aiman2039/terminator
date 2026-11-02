//! #83: type-ahead is a GUI preference that applies to shell terminals only.
use super::*;
use crate::preferences::TypeAhead;

#[test]
fn type_ahead_follows_the_preference_for_shells_only() {
    let (mut app, _ctx, _dir) = fixture();
    app.state.sessions = vec![
        session_fixture("shell", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    assert_eq!(app.preferences.type_ahead, TypeAhead::Auto);
    assert_eq!(app.type_ahead_mode("shell"), egui_term::TypeAheadMode::Auto);
    assert_eq!(app.type_ahead_mode("edit"), egui_term::TypeAheadMode::Off);
    app.preferences.type_ahead = TypeAhead::Off;
    assert_eq!(app.type_ahead_mode("shell"), egui_term::TypeAheadMode::Off);
}

#[test]
fn saved_preferences_without_type_ahead_default_to_auto() {
    let saved: crate::preferences::UiPreferences =
        serde_json::from_str(r#"{"version":2}"#).unwrap();
    assert_eq!(saved.type_ahead, TypeAhead::Auto);
    let on: crate::preferences::UiPreferences =
        serde_json::from_str(r#"{"version":2,"type_ahead":"on"}"#).unwrap();
    assert_eq!(on.type_ahead, TypeAhead::On);
}
