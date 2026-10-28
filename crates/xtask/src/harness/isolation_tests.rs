use super::*;
fn gui_env(command: &Command, name: &str) -> Option<std::ffi::OsString> {
    command
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new(name))
        .and_then(|(_, value)| value.map(std::borrow::ToOwned::to_owned))
}

#[test]
fn gui_defaults_to_glow_without_inherited_foreground_or_native_input() {
    let mut command = Command::new("fixture");
    for key in [
        "TERMINATOR_TEST_VISIBLE_CAPTURE",
        "TERMINATOR_TEST_FOREGROUND",
        "TERMINATOR_TEST_NATIVE_INPUT",
    ] {
        command.env(key, "1");
    }
    configure_gui(&mut command, None, false);
    assert_eq!(
        gui_env(&command, "TERMINATOR_RENDERER"),
        Some("glow".into())
    );
    assert_eq!(
        gui_env(&command, "TERMINATOR_TEST_BACKGROUND"),
        Some("1".into())
    );
    assert_eq!(
        gui_env(&command, "TERMINATOR_TEST_RENDER_OCCLUDED"),
        Some("1".into())
    );
    for key in [
        "TERMINATOR_TEST_VISIBLE_CAPTURE",
        "TERMINATOR_TEST_FOREGROUND",
        "TERMINATOR_TEST_NATIVE_INPUT",
    ] {
        assert_eq!(gui_env(&command, key), None);
    }
}

#[test]
fn explicit_visible_renderer_test_preserves_renderer_selection() {
    let mut command = Command::new("fixture");
    command.env("TERMINATOR_TEST_BACKGROUND", "1");
    configure_gui(&mut command, Some("wgpu".into()), true);
    assert_eq!(
        gui_env(&command, "TERMINATOR_RENDERER"),
        Some("wgpu".into())
    );
    assert_eq!(gui_env(&command, "TERMINATOR_TEST_BACKGROUND"), None);
}

#[test]
fn closed_session_accepts_ended_history_or_a_pruned_record() {
    for state in [
        json!({"sessions":[]}),
        json!({"sessions":[{"id":"editor","lifecycle":"ended"}]}),
    ] {
        assert!(session_closed(&state, "editor"));
    }
}

#[test]
fn closed_session_rejects_live_stopping_and_unknown_records() {
    for lifecycle in ["running", "stopping", "starting", "unknown"] {
        let state = json!({"sessions":[
            {"id":"other","lifecycle":"ended"},
            {"id":"editor","lifecycle":lifecycle}
        ]});
        assert!(!session_closed(&state, "editor"), "Accepted {lifecycle}");
    }
    assert!(!session_closed(
        &json!({"sessions":[{"id":"editor"}]}),
        "editor"
    ));
}

#[test]
fn inherited_catalog_generation_and_session_routing_cannot_escape_fixture() {
    let mut command = Command::new("fixture");
    let names = [
        "TERMINATOR_CATALOG_DATA",
        "TERMINATOR_CATALOG_RUNTIME",
        "TERMINATOR_GENERATION",
        "TERMINATOR_DATA_DIR",
        "TERMINATOR_RUNTIME_DIR",
        "TERMINATOR_SESSION_ID",
        "TERMINATOR_SESSION_TOKEN",
    ];
    clear_inherited_terminator(&mut command, names.iter().map(std::ffi::OsString::from));
    command.env("TERMINATOR_DATA_DIR", "/isolated/fixture");
    for name in names {
        let value = command
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new(name))
            .unwrap()
            .1;
        if name == "TERMINATOR_DATA_DIR" {
            assert_eq!(value, Some(std::ffi::OsStr::new("/isolated/fixture")));
        } else {
            assert!(value.is_none(), "inherited route {name} was retained");
        }
    }
}
