use super::super::*;
use super::tests::support::{drain_updates, fixture, session_fixture};
use std::{path::PathBuf, sync::mpsc};
use terminator_core::*;

fn panes(app: &App, project: &str) -> usize {
    app.layouts[project]
        .tabs
        .iter()
        .map(|group| group.layout.iter_all_tabs().count())
        .sum()
}

#[test]
fn repeated_native_opens_reveal_the_existing_split_and_preserve_the_buffer() {
    let (mut app, ctx, _dir) = fixture();
    app.state.settings.editor_mode = EditorMode::Native;
    let path = crate::test_path("/a/file.rs");
    app.native_docs.insert(
        path.clone(),
        native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let first = Tab::NativeEditor {
        path: "/a/other.rs".into(),
    };
    let requested = Tab::NativeEditor { path: path.clone() };
    let mut layout = egui_dock::DockState::new(vec![first]);
    let right = layout.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Player, requested.clone()],
    )[1];
    app.layouts
        .insert("a".into(), Workspace::from_layout(layout));
    let original = app.layouts["a"].active.clone();
    app.layouts.get_mut("a").unwrap().add(
        "other".into(),
        Tab::Image {
            path: "/a/image.png".into(),
        },
    );
    for _ in 0..2 {
        app.open_file(path.clone(), Some(8), None, false);
    }
    drain_updates(&mut app, &ctx);
    assert_eq!(app.layouts["a"].tabs.len(), 2);
    assert_eq!(panes(&app, "a"), 4);
    assert_eq!(app.layouts["a"].active, original);
    assert_eq!(app.layouts["a"].main_surface().focused_leaf(), Some(right));
    assert_eq!(app.layouts["a"].active_pane(), Some(&requested));
    assert_eq!(app.native_pending_line.get(&path), Some(&7));
    assert_eq!(app.native_docs.len(), 1, "the dirty buffer must be reused");
}

#[test]
fn queued_first_opens_create_one_view_but_explicit_splits_create_another() {
    let (mut app, ctx, _dir) = fixture();
    app.state.settings.editor_mode = EditorMode::Native;
    let path = PathBuf::from("/a/file.rs");
    for _ in 0..2 {
        app.open_file(path.clone(), None, None, false);
    }
    drain_updates(&mut app, &ctx);
    assert_eq!(panes(&app, "a"), 1);
    app.open_file(path, None, Some("right"), false);
    drain_updates(&mut app, &ctx);
    assert_eq!(panes(&app, "a"), 2);
    assert_eq!(app.layouts["a"].tabs.len(), 1);
}

#[test]
fn repeated_image_and_html_opens_keep_the_existing_view_identity() {
    let (mut app, ctx, _dir) = fixture();
    for _ in 0..2 {
        app.open_image("a", "/a/image.png".into(), None);
        app.open_html("a", "/a/index.html".into(), None);
    }
    drain_updates(&mut app, &ctx);
    assert_eq!(app.layouts["a"].tabs.len(), 2);
    assert_eq!(panes(&app, "a"), 2);
    let browser = app.layouts["a"].active_pane().cloned().unwrap();
    app.open_html("a", "/a/index.html".into(), None);
    drain_updates(&mut app, &ctx);
    assert_eq!(app.layouts["a"].active_pane(), Some(&browser));
}

#[test]
fn repeated_diff_opens_focus_the_existing_leaf_and_keep_staged_views_distinct() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let working = Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
    };
    let staged = Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: true,
    };
    let mut layout = egui_dock::DockState::new(vec![Tab::Player]);
    let right = layout.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![
            Tab::Image {
                path: "/a/image.png".into(),
            },
            working.clone(),
        ],
    )[1];
    app.layouts
        .insert("a".into(), Workspace::from_layout(layout));
    app.spawn_diff(SpawnDiff {
        cwd: "/a".into(),
        path: "file.rs".into(),
        staged: false,
        native: true,
    });
    assert_eq!(app.layouts["a"].active_pane(), Some(&working));
    assert_eq!(app.layouts["a"].main_surface().focused_leaf(), Some(right));
    assert!(
        requests.try_recv().is_err(),
        "reopening a diff must not queue another view"
    );
    app.spawn_diff(SpawnDiff {
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: true,
        native: true,
    });
    assert_eq!(app.layouts["a"].tabs.len(), 2);
    assert_eq!(app.layouts["a"].active_pane(), Some(&staged));
}

#[test]
fn opening_a_floated_file_reveals_its_window_without_a_docked_copy() {
    let (mut app, ctx, _dir) = fixture();
    let viewport = egui::ViewportId::from_hash_of("file-window");
    let path = crate::test_path("/a/file.rs");
    let tab = Tab::NativeEditor { path: path.clone() };
    app.state.settings.editor_mode = EditorMode::Native;
    app.floating.push(FloatingWindow {
        viewport,
        home: ("a".into(), "home".into()),
        dock: Some(egui_dock::DockState::new(vec![tab.clone()])),
    });
    app.open_file(path, None, None, false);
    drain_updates(&mut app, &ctx);
    assert_eq!(panes(&app, "a"), 0);
    assert_eq!(app.floating.len(), 1);
    assert_eq!(app.focus_float_tab, Some((viewport, tab)));
    assert_eq!(
        app.floating[0]
            .dock
            .as_ref()
            .unwrap()
            .iter_all_tabs()
            .count(),
        1
    );
}

#[test]
fn opening_a_daemon_editor_focuses_its_session_without_creating_another() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.settings.editor_mode = EditorMode::Embedded;
    let mut session = session_fixture("editor", SessionKind::Editor);
    session.file = Some("/a/file.rs".into());
    app.state.sessions.push(session);
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("editor-tab".into(), Tab::Terminal("editor".into()));
    app.layouts.get_mut("a").unwrap().add(
        "other-tab".into(),
        Tab::Image {
            path: "/a/image.png".into(),
        },
    );
    app.open_file("/a/file.rs".into(), None, None, false);
    drain_updates(&mut app, &ctx);
    assert_eq!(app.layouts["a"].tabs.len(), 2);
    assert_eq!(app.active_session.as_deref(), Some("editor"));
    assert!(!requests.try_iter().any(
        |job| matches!(job, Job::Control(request, _) if matches!(*request, Request::Create { .. }))
    ));
}
