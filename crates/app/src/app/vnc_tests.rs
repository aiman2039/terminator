use super::super::*;
use super::nav_common::*;
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

fn float_window(name: &str, project: &str, home: &str, tabs: Vec<Tab>) -> FloatingWindow {
    FloatingWindow {
        viewport: egui::ViewportId::from_hash_of(name),
        dock: Some(DockState::new(tabs)),
        home: (project.into(), home.into()),
    }
}

fn vnc_session(id: &str, status: VncSessionStatus) -> VncSession {
    VncSession {
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
    app.floating
        .push(float_window("float-vnc", "a", "home", vec![float.clone()]));
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
