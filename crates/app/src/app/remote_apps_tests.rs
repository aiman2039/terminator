use super::super::*;
use super::tests::support::*;
use crate::remote_apps_ui::RemoteAppsFrame;
use terminator_core::remote_apps::{
    RemoteAppsState, RemoteAppsStatus, RemoteWindow, WindowId, WindowRole,
};

fn window(id: u64, title: &str, role: WindowRole) -> RemoteWindow {
    RemoteWindow {
        id: WindowId(id),
        role,
        title: title.into(),
        app_id: String::new(),
        width: 0,
        height: 0,
        revision: 1,
        shm: None,
        sequence: 0,
    }
}

fn state(status: RemoteAppsStatus, windows: Vec<RemoteWindow>) -> Box<RemoteAppsFrame> {
    Box::new(RemoteAppsFrame {
        state: RemoteAppsState {
            session: "s1".into(),
            project_id: "a".into(),
            status,
            error: None,
            error_kind: None,
            display: Some(":100".into()),
            revision: 5,
            retry_attempt: 0,
            retry_in_seconds: 0,
            windows,
        },
        images: Vec::new(),
        torn: false,
    })
}

fn remote_tabs(app: &App) -> Vec<(u64, String)> {
    let mut tabs: Vec<(u64, String)> = app
        .layouts
        .get("a")
        .into_iter()
        .flat_map(|w| w.tabs.iter())
        .flat_map(|tab| tab.layout.iter_all_tabs())
        .filter_map(|(_, pane)| match pane {
            Tab::RemoteApp { window, title, .. } => Some((*window, title.clone())),
            _ => None,
        })
        .collect();
    tabs.sort();
    tabs
}

fn connected(app: &mut App, ctx: &egui::Context, windows: Vec<RemoteWindow>) {
    app.handle_remote_apps(
        ctx,
        "a",
        Ok(Some(state(RemoteAppsStatus::Connected, windows))),
    );
}

#[test]
fn each_toplevel_window_opens_one_tab_and_popups_open_none() {
    let (mut app, ctx, _dir) = fixture();
    app.connect_remote_apps("a");
    let popup = WindowRole::Popup {
        parent: WindowId(7),
        x: 1,
        y: 2,
    };
    connected(
        &mut app,
        &ctx,
        vec![
            window(7, "xclock", WindowRole::Toplevel),
            window(9, "", popup),
        ],
    );
    connected(
        &mut app,
        &ctx,
        vec![window(7, "xclock", WindowRole::Toplevel)],
    );
    assert_eq!(remote_tabs(&app), [(7, "xclock".to_owned())]);
}

#[test]
fn title_change_renames_the_tab_and_a_closed_window_closes_it() {
    let (mut app, ctx, _dir) = fixture();
    app.connect_remote_apps("a");
    connected(&mut app, &ctx, vec![window(7, "a", WindowRole::Toplevel)]);
    connected(&mut app, &ctx, vec![window(7, "b", WindowRole::Toplevel)]);
    assert_eq!(remote_tabs(&app), [(7, "b".to_owned())]);
    connected(&mut app, &ctx, Vec::new());
    assert!(remote_tabs(&app).is_empty());
}

#[test]
fn tabs_stay_while_the_session_reconnects() {
    let (mut app, ctx, _dir) = fixture();
    app.connect_remote_apps("a");
    connected(
        &mut app,
        &ctx,
        vec![window(7, "xclock", WindowRole::Toplevel)],
    );
    app.handle_remote_apps(
        &ctx,
        "a",
        Ok(Some(state(RemoteAppsStatus::Connecting, Vec::new()))),
    );
    assert_eq!(remote_tabs(&app), [(7, "xclock".to_owned())]);
}

#[test]
fn closing_a_tab_asks_the_app_while_connected_and_closes_it_otherwise() {
    let (mut app, ctx, _dir) = fixture();
    assert!(!app.remote_app_close_requested("a", 7));
    app.connect_remote_apps("a");
    connected(
        &mut app,
        &ctx,
        vec![window(7, "xclock", WindowRole::Toplevel)],
    );
    assert!(app.remote_app_close_requested("a", 7));
    app.handle_remote_apps(
        &ctx,
        "a",
        Ok(Some(state(RemoteAppsStatus::Failed, Vec::new()))),
    );
    assert!(!app.remote_app_close_requested("a", 7));
}

#[test]
fn a_forgotten_session_reconnects_after_a_poll_error() {
    let (mut app, ctx, _dir) = fixture();
    app.connect_remote_apps("a");
    connected(&mut app, &ctx, Vec::new());
    assert!(app.remote_apps_connected("a"));
    app.handle_remote_apps(&ctx, "a", Err("Unknown remote-apps session".into()));
    assert!(!app.remote_apps_connected("a"));
}
