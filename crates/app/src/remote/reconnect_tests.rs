use crate::app::tests::support::{fixture, session_fixture};
use crate::workspace::Workspace;
use crate::{After, App, Job, Tab};
use eframe::egui;
use egui_dock::DockState;
use std::sync::mpsc;
use terminator_core::{
    Lifecycle, Request, SessionKind, State,
    remote::{RemoteProject, RemoteSession},
    ssh::SshOptions,
};

fn host(mosh: bool) -> RemoteProject {
    RemoteProject {
        id: "h".into(),
        name: "Host".into(),
        ssh: SshOptions {
            host: "fixture".into(),
            ..SshOptions::default()
        },
        root: "/srv".into(),
        layout: serde_json::Value::Null,
        group: None,
        forwards: Vec::new(),
        last_connected: 0,
        command_timeout_seconds: None,
        mosh,
        shell_command: Some("tmux new -A -s main".into()),
        mosh_command: None,
        apps_command: None,
    }
}

fn dropped(sid: &str) -> terminator_core::Session {
    let mut session = session_fixture(sid, SessionKind::Shell);
    session.project_id = "h".into();
    session.lifecycle = Lifecycle::Ended;
    session.exit_code = Some(255);
    session.pid = None;
    session.remote = Some(RemoteSession {
        ssh: host(false).ssh,
        root: "/srv".into(),
        cwd: "/srv/app".into(),
        editor_socket: None,
    });
    session
}

/// An SSH terminal whose connection dropped, beside a live local shell.
fn lost(mosh: bool) -> (App, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, _ctx, dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    let mut state: State = app.state.clone();
    state.remote_projects = vec![host(mosh)];
    state.sessions = vec![session_fixture("left", SessionKind::Shell), dropped("old")];
    app.layouts.insert(
        "h".into(),
        Workspace::from_layout(DockState::new(vec![
            Tab::Terminal("left".into()),
            Tab::Terminal("old".into()),
        ])),
    );
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    (app, dir, received)
}

fn sent(received: &mpsc::Receiver<Job>) -> Vec<(Request, Option<String>)> {
    let mut out = Vec::new();
    while let Ok(job) = received.try_recv() {
        if let Job::Control(request, after) = job {
            let old = match after {
                After::Restore(old) => Some(old),
                _ => None,
            };
            out.push((*request, old));
        }
    }
    out
}

#[test]
fn reconnect_never_runs_without_the_click() {
    let (mut app, _dir, received) = lost(false);
    let session = dropped("old");
    assert!(app.ssh_connection_lost(&session));
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.reconnect_bar(ui, &session);
        });
        output.textures_delta.clear();
    }
    assert!(
        sent(&received)
            .iter()
            .all(|(request, _)| !matches!(request, Request::Create { .. } | Request::Input { .. })),
        "showing the bar must not open or type anything"
    );
}

#[test]
fn reconnect_click_opens_a_new_ssh_terminal_in_the_same_pane() {
    let (mut app, _dir, received) = lost(false);
    app.reconnect_ssh(&dropped("old"));
    let creates = sent(&received);
    let [(request, old)] = creates.as_slice() else {
        panic!("one request: {creates:?}");
    };
    let Request::Create {
        project,
        cwd,
        file,
        editor,
        ..
    } = request
    else {
        panic!("a new terminal: {request:?}");
    };
    assert_eq!(project, "h");
    assert_eq!(cwd.as_deref(), Some(std::path::Path::new("/srv/app")));
    assert!(file.is_none() && !editor);
    assert_eq!(old.as_deref(), Some("old"));
    // The reply puts the new terminal where the dead one was and drops
    // the dead record.
    let mut fresh = session_fixture("fresh", SessionKind::Shell);
    fresh.project_id = "h".into();
    app.handle_restored(fresh, "old");
    let tabs: Vec<Tab> = app.layouts["h"].tabs[0]
        .layout
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .collect();
    assert_eq!(
        tabs,
        [Tab::Terminal("left".into()), Tab::Terminal("fresh".into())]
    );
    assert!(
        sent(&received)
            .iter()
            .any(|(request, _)| matches!(request, Request::Remove { session } if session == "old"))
    );
}

#[test]
fn mosh_terminals_get_no_reconnect() {
    let (mut app, _dir, received) = lost(true);
    let session = dropped("old");
    assert!(!app.ssh_connection_lost(&session));
    app.reconnect_ssh(&session);
    assert!(sent(&received).is_empty());
}
