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
        created: 0,
        generation: "fixture".into(),
    }
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
fn a_closed_vnc_tab_disconnects_only_when_no_copy_remains() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    // Two tabs still show the session: the close must keep it connected.
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![
            vnc_tab("t1", "s1", "dev"),
            vnc_tab("t2", "s1", "dev"),
        ])),
    );
    app.pending_vnc_disconnect = Some("s1".into());
    app.drain_pending_vnc_disconnect();
    assert!(
        requests.try_recv().is_err(),
        "a remaining tab keeps the remote session"
    );
    assert_eq!(app.pending_vnc_disconnect, None, "the request is consumed");

    // The last copy is gone: the disconnect lands exactly once.
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![vnc_tab("t1", "other", "dev")])),
    );
    app.pending_vnc_disconnect = Some("s1".into());
    app.drain_pending_vnc_disconnect();
    match requests.try_recv() {
        Ok(Job::Control(request, _)) => assert!(
            matches!(*request, Request::VncDisconnect { session } if session == "s1"),
            "the deferred close disconnects the session"
        ),
        _ => panic!("expected one VNC disconnect request"),
    }
    assert!(
        requests.try_recv().is_err(),
        "a drained close never queues a second disconnect"
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
                    clipboard: Some(text.into()),
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
