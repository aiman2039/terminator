use super::super::*;
use super::tests::support::*;
use crate::vnc_ui::VncViewState;
use std::sync::mpsc;
use terminator_core::vnc::{VncSession, VncSessionStatus};

fn vnc_tab(id: &str, session: &str, label: &str) -> Tab {
    Tab::Vnc {
        id: id.into(),
        session: session.into(),
        profile_id: None,
        host: "127.0.0.1".into(),
        port: 5900,
        label: label.into(),
    }
}

struct FloatWindowInput<'a> {
    name: &'a str,
    project: &'a str,
    home: &'a str,
    tabs: Vec<Tab>,
}

fn float_window(input: FloatWindowInput<'_>) -> FloatingWindow {
    let FloatWindowInput {
        name,
        project,
        home,
        tabs,
    } = input;
    FloatingWindow {
        viewport: egui::ViewportId::from_hash_of(name),
        dock: Some(DockState::new(tabs)),
        home: (project.into(), home.into()),
    }
}

fn vnc_session(id: &str, status: VncSessionStatus) -> VncSession {
    VncSession {
        options: terminator_core::vnc::VncOptions::default(),
        retry_attempt: 0,
        retry_in_seconds: 0,
        id: id.into(),
        project_id: "a".into(),
        profile_id: None,
        label: id.into(),
        host: "127.0.0.1".into(),
        port: 5900,
        width: 0,
        height: 0,
        view_only: false,
        status,
        error: None,
        error_kind: None,
        created: 0,
        generation: "fixture".into(),
    }
}

#[test]
fn a_password_required_failure_opens_the_password_prompt_once() {
    let (mut app, _ctx, _dir) = fixture();
    let mut session = vnc_session("s1", VncSessionStatus::Failed);
    session.error_kind = Some(terminator_core::vnc::VncErrorKind::PasswordRequired);
    app.vnc_auto_password_prompt("s1", &session);
    let view = app.vnc_views.get_mut("s1").expect("the prompt view exists");
    assert!(!view.save_profile, "the pane asks for the password");
    assert_eq!(view.draft_port, "5900");
    // The user went back to all hosts: the same failure must not force it again.
    view.save_profile = true;
    app.vnc_auto_password_prompt("s1", &session);
    assert!(app.vnc_views.get("s1").is_some_and(|v| v.save_profile));
}

#[test]
fn pruning_keeps_view_state_for_open_tabs_and_drafts_only() {
    let (mut app, _ctx, _dir) = fixture();
    for key in ["s1", "draft:t1", "closed"] {
        app.vnc_views.insert(key.into(), VncViewState::default());
    }
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![
            vnc_tab("t1", "", ""),
            vnc_tab("t2", "s1", "dev"),
        ])),
    );
    app.prune_vnc_views();
    assert!(
        app.vnc_views.contains_key("s1"),
        "an open tab keeps its session view"
    );
    assert!(
        app.vnc_views.contains_key("draft:t1"),
        "a tab that is still connecting keeps its draft view"
    );
    assert!(
        !app.vnc_views.contains_key("closed"),
        "view state for a closed tab is dropped"
    );

    // A float counts as an open tab on its own.
    let float = vnc_tab("t3", "s1", "dev");
    app.floating.push(float_window(FloatWindowInput {
        name: "float-vnc",
        project: "a",
        home: "home",
        tabs: vec![float.clone()],
    }));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![vnc_tab("t2", "other", "")])),
    );
    app.vnc_views.insert("s1".into(), VncViewState::default());
    app.prune_vnc_views();
    assert!(
        app.vnc_views.contains_key("s1"),
        "a floated tab keeps the session view alive"
    );

    // A view painted this frame survives while its tab is being re-homed.
    app.vnc_views
        .insert("midmove".into(), VncViewState::default());
    app.visible_vnc.insert("midmove".into());
    app.prune_vnc_views();
    assert!(
        app.vnc_views.contains_key("midmove"),
        "a painted view is kept even with no tab found yet"
    );
    app.visible_vnc.clear();
    app.prune_vnc_views();
    assert!(
        !app.vnc_views.contains_key("midmove"),
        "the next frame with no tab drops it"
    );
}

#[test]
fn a_detached_vnc_session_is_reported_without_a_tab() {
    let (mut app, _ctx, _dir) = fixture();
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![vnc_tab("t1", "s1", "dev")])),
    );
    assert!(
        app.vnc_session_has_tab("s1"),
        "an open tab shows the session"
    );
    assert!(
        !app.vnc_session_has_tab("s2"),
        "a session whose tab closed is detached, not disconnected"
    );
}

#[test]
fn the_tab_face_carries_the_vnc_connection_state() {
    let (mut app, _ctx, _dir) = fixture();
    app.state.vnc_sessions = vec![vnc_session("s1", VncSessionStatus::Connected)];
    let face = app.tab_face(Some(&vnc_tab("t1", "s1", "dev")));
    assert_eq!(face.label, "dev");
    let (state, icon, _) = face.status.expect("a live session shows its state");
    assert_eq!(state, AgentState::Running);
    assert_eq!(icon, "Circle");

    app.state.vnc_sessions = vec![vnc_session("s1", VncSessionStatus::Connecting)];
    let (state, icon, _) = app
        .tab_face(Some(&vnc_tab("t1", "s1", "dev")))
        .status
        .expect("a connecting session shows its state");
    assert_eq!(state, AgentState::Running);
    assert_eq!(icon, "LoaderCircle", "connecting spins");

    app.state.vnc_sessions = vec![vnc_session("s1", VncSessionStatus::Failed)];
    let (state, icon, _) = app
        .tab_face(Some(&vnc_tab("t1", "s1", "dev")))
        .status
        .expect("a failed session shows its state");
    assert_eq!(state, AgentState::Failed);
    assert_eq!(icon, "CircleAlert");

    // A draft has no session yet: the kind glyph and the endpoint stand in.
    let draft = app.tab_face(Some(&vnc_tab("t1", "", "")));
    assert!(draft.status.is_none());
    assert_eq!(draft.icon, "Monitor");
    assert_eq!(draft.label, "127.0.0.1:5900");
}

#[test]
fn save_and_connect_uses_the_password_without_storing_it() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.vnc_sidebar.begin_new();
    app.vnc_sidebar.password = "fixture-session-password".into();
    app.vnc_sidebar.save_password = false;
    app.save_vnc_profile_form(true);
    let saved_id = match requests.try_recv().unwrap() {
        Job::Control(request, _) => match *request {
            Request::VncProfileSave { profile, .. } => {
                assert!(profile.password.is_empty());
                profile.id
            }
            _ => panic!("expected profile save"),
        },
        _ => panic!("expected control request"),
    };
    match requests.try_recv().unwrap() {
        Job::Control(request, _) => match *request {
            Request::VncConnect {
                password,
                profile_id,
                shared,
                view_only,
                ..
            } => {
                assert_eq!(password, "fixture-session-password");
                assert_eq!(profile_id.as_deref(), Some(saved_id.as_str()));
                assert!(shared);
                assert!(!view_only);
            }
            _ => panic!("expected connection"),
        },
        _ => panic!("expected control request"),
    }
    assert!(app.vnc_sidebar.password.is_empty());
    assert!(!app.vnc_sidebar.form_open);
}

#[test]
fn invalid_vnc_port_keeps_the_form_open_without_sending_requests() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    for port in ["0", "65536", "oops", ""] {
        app.vnc_sidebar.begin_new();
        app.vnc_sidebar.port = port.into();
        app.save_vnc_profile_form(true);
        assert!(app.vnc_sidebar.form_open);
        assert!(app.vnc_sidebar.error.is_some());
        assert!(requests.try_recv().is_err());
    }
}

#[test]
fn terminal_title_style_survives_preferences_reload() {
    let (mut app, _ctx, dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("styled", SessionKind::Shell));
    let style = app
        .preferences
        .terminal_styles
        .entry("styled".into())
        .or_default();
    style.icon = Some("Database".into());
    style.color = Some("#BB9AF7".into());
    app.preferences.save(dir.path()).unwrap();
    app.preferences = UiPreferences::load(dir.path()).unwrap();
    let face = app.tab_face(Some(&Tab::Terminal("styled".into())));
    assert_eq!(face.icon, "Database");
    assert_eq!(
        app.preferences.terminal_styles["styled"].color.as_deref(),
        Some("#BB9AF7")
    );
    assert!(!app.preferences.terminal_styles.contains_key("other"));
}

#[test]
fn repeated_vnc_frames_do_not_overwrite_a_new_local_clipboard() {
    let (mut app, ctx, _dir) = fixture();
    let mut receive = |text: &str| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
            app.apply_vnc_frame(
                &ctx,
                crate::vnc_ui::VncDecodedFrame {
                    cursor: None,
                    monitors: Vec::new(),
                    resize_supported: false,
                    session: "clipboard-session".into(),
                    width: 1,
                    height: 1,
                    revision: 1,
                    status: VncSessionStatus::Connected,
                    error: None,
                    image: None,
                    region: None,
                    patches: Vec::new(),
                    cursor_serial: 0,
                    shm_failed: false,
                    clipboard: Some(text.into()),
                    stats: None,
                },
            );
        });
        output.textures_delta.clear();
        output
            .platform_output
            .commands
            .into_iter()
            .filter_map(|command| {
                if let egui::OutputCommand::CopyText(text) = command {
                    Some(text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(receive("remote text"), ["remote text"]);
    assert!(receive("remote text").is_empty());
    assert_eq!(receive("new remote text"), ["new remote text"]);
    assert_eq!(receive(""), [""]);
    assert!(receive("").is_empty());
}

#[test]
fn hidden_vnc_views_do_not_capture_application_shortcuts() {
    let (mut app, ctx, _dir) = fixture();
    let mut id = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let response = ui.button("Remote focus fixture");
        response.request_focus();
        id = Some(response.id);
    });
    output.textures_delta.clear();
    app.vnc_views.insert(
        "focus-fixture".into(),
        VncViewState {
            focused: true,
            capture_shortcuts: true,
            focus_id: id,
            visible_last_frame: true,
            ..Default::default()
        },
    );
    assert!(app.vnc_captures_shortcuts(&ctx));
    app.vnc_views
        .get_mut("focus-fixture")
        .unwrap()
        .visible_last_frame = false;
    assert!(!app.vnc_captures_shortcuts(&ctx));
}

#[test]
fn connecting_a_host_with_a_live_session_reattaches_it() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let mut session = vnc_session("s1", VncSessionStatus::Connected);
    session.profile_id = Some("p1".into());
    app.state.vnc_sessions = vec![session];
    let profile = terminator_core::vnc::VncProfile {
        id: "p1".into(),
        name: "dev".into(),
        host: "127.0.0.1".into(),
        port: 5900,
        username: String::new(),
        password: String::new(),
        shared: true,
        view_only: false,
        last_connected: 0,
        options: Default::default(),
        credential_saved: true,
        labels: Vec::new(),
    };
    app.connect_vnc_profile(&profile, None);
    while let Ok(job) = requests.try_recv() {
        if let Job::Control(request, _) = job {
            assert!(
                !matches!(*request, Request::VncConnect { .. }),
                "a live session must not get a second connection"
            );
        }
    }
}

#[test]
fn quitting_with_live_vnc_sessions_asks_and_can_disconnect_them() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.vnc_sessions = vec![
        vnc_session("live", VncSessionStatus::Connected),
        vnc_session("gone", VncSessionStatus::Disconnected),
    ];
    assert!(!app.vnc_quit_allowed(), "live sessions ask first");
    assert!(app.vnc_quit_prompt);

    app.answer_vnc_quit(&ctx, crate::vnc_ui::QuitChoice::Cancel);
    assert!(!app.vnc_quit_prompt && !app.vnc_quit_confirmed);
    assert!(requests.try_recv().is_err(), "cancel sends nothing");

    assert!(!app.vnc_quit_allowed());
    app.answer_vnc_quit(&ctx, crate::vnc_ui::QuitChoice::Disconnect);
    match requests.try_recv() {
        Ok(Job::Control(request, _)) => assert!(
            matches!(*request, Request::VncDisconnect { session } if session == "live"),
            "only the live session is disconnected"
        ),
        _ => panic!("expected one VNC disconnect"),
    }
    assert!(app.vnc_quit_allowed(), "answered once per quit");
}

#[test]
fn leaving_vnc_fullscreen_restores_shortcut_capture() {
    let (mut app, ctx, _dir) = fixture();
    let view = app.vnc_views.entry("s1".into()).or_default();
    view.capture_before_fullscreen = Some(false);
    view.capture_shortcuts = true;
    app.vnc_fullscreen = Some(vnc_tab("t1", "s1", "dev"));
    app.exit_vnc_fullscreen(&ctx);
    let view = &app.vnc_views["s1"];
    assert!(app.vnc_fullscreen.is_none());
    assert!(
        !view.capture_shortcuts,
        "capture goes back to its old value"
    );
    assert!(view.capture_before_fullscreen.is_none());
}
