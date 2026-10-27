use super::*;

fn tab_native_paths(app: &App, project: &str, tab: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(workspace) = app.layouts.get(project)
        && let Some(tab_state) = workspace.tabs.iter().find(|tab_state| tab_state.id == tab)
    {
        for (_, tab) in tab_state.layout.iter_all_tabs() {
            if let Tab::NativeEditor { path } = tab {
                paths.push(path.clone());
            }
        }
    }
    paths
}

fn docked_copies(app: &App, project: &str, path: &Path) -> usize {
    app.layouts
        .get(project)
        .map(|workspace| {
            workspace
                .tabs
                .iter()
                .flat_map(|tab| tab.layout.iter_all_tabs())
                .filter(|(_, tab)| {
                    matches!(tab, Tab::NativeEditor { path: current } if current == path)
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn main_dock_native_split_fans_out_from_the_issuing_view() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/main-split.rs");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("native".into(), Tab::NativeEditor { path: path.clone() });
    app.selected = Some("a".into());
    let views = |app: &App| {
        app.layouts["a"]
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.iter_all_tabs())
            .filter(|(_, tab)| matches!(tab, Tab::NativeEditor { .. }))
            .count()
    };
    for _ in 0..2 {
        app.pending_native_splits.push((
            egui::ViewportId::ROOT,
            path.clone(),
            crate::native_editor::NativeSplit::Beside,
        ));
        app.drain_pending_native_close();
    }
    assert_eq!(views(&app), 3, "each :vsplit adds one view of the file");
}

#[test]
fn main_dock_native_tabnew_adds_a_top_level_tab() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/main-tabnew.rs");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("native".into(), Tab::NativeEditor { path: path.clone() });
    app.selected = Some("a".into());
    let tabs = app.layouts["a"].tabs.len();
    app.pending_native_splits.push((
        egui::ViewportId::ROOT,
        path.clone(),
        crate::native_editor::NativeSplit::NewTab,
    ));
    app.drain_pending_native_close();
    assert_eq!(app.layouts["a"].tabs.len(), tabs + 1);
}

#[test]
fn dock_close_button_names_exact_pane() {
    use egui_dock::TabViewer;
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/exact-pane.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let left = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: egui_dock::NodeIndex(0),
    };
    let right = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: egui_dock::NodeIndex(1),
    };
    // Main dock: the clicked leaf travels into the prompt, not a
    // focus sample.
    let mut main = crate::workspace_ui::Viewer {
        app: &mut app,
        strip: false,
        project: Some("a".into()),
        tab: Some("tab".into()),
        window: None,
        render_path: None,
        render_tab: None,
    };
    let mut tab = Tab::NativeEditor { path: path.clone() };
    assert_eq!(
        main.on_close(&mut tab, left),
        crate::OnCloseResponse::Ignore
    );
    assert!(matches!(
        &main.app.native_close_prompt,
        Some((prompt, Some(crate::native_editor::CloseIssuer::Docked { node, .. })))
            if prompt == &path && *node == Some(left)
    ));
    // Floating dock: same, through the window issuer.
    let viewport = egui::ViewportId::from_hash_of("exact-pane-float");
    let mut float = crate::workspace_ui::Viewer {
        app: main.app,
        strip: false,
        project: Some("a".into()),
        tab: None,
        window: Some(viewport),
        render_path: None,
        render_tab: None,
    };
    let mut tab = Tab::NativeEditor { path: path.clone() };
    assert_eq!(
        float.on_close(&mut tab, right),
        crate::OnCloseResponse::Ignore
    );
    assert!(matches!(
        &float.app.native_close_prompt,
        Some((prompt, Some(crate::native_editor::CloseIssuer::Floating { node, .. })))
            if prompt == &path && *node == Some(right)
    ));
}

#[test]
fn plain_close_keeps_other_docked_copies() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/shared.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    // Two splits on the same file in `a`, one copy in `b`.
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::NativeEditor { path: path.clone() }],
    );
    app.layouts.insert("a".into(), workspace);
    app.layouts.insert(
        "b".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }])),
    );
    // A plain close issued in `a` removes one copy there; the other
    // split and project `b` keep theirs, and the buffer survives.
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    app.close_native_tab(&path, false, docked_issuer("a", &tab, None, false));
    assert_eq!(docked_copies(&app, "a", &path), 1);
    assert_eq!(docked_copies(&app, "b", &path), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn plain_close_takes_focused_split_not_first_match() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/focused.md");
    let other = PathBuf::from("/a/other.md");
    for file in [&path, &other] {
        app.native_docs.insert(
            file.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(file.clone()),
        );
    }
    // Left leaf holds only the file; the focused right leaf holds
    // the file plus another tab.
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    let [_, right] = workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![
            Tab::NativeEditor {
                path: other.clone(),
            },
            Tab::NativeEditor { path: path.clone() },
        ],
    );
    workspace.tabs[0]
        .layout
        .set_focused_node_and_surface(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: right,
        });
    app.layouts.insert("a".into(), workspace);
    // `:q` in the focused split removes that copy: the focused leaf
    // keeps only the other tab, the first leaf keeps the file. No
    // recorded pane here, so the focused copy goes.
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    app.close_native_tab(&path, false, docked_issuer("a", &tab, None, false));
    let workspace = app.layouts.get("a").expect("workspace");
    let mut focused_tabs = Vec::new();
    let mut copies = 0;
    for (at, tab) in workspace.tabs[0].layout.iter_all_tabs() {
        if matches!(tab, Tab::NativeEditor { path: current } if current.as_path() == path.as_path())
        {
            copies += 1;
        }
        if at.node == right
            && let Tab::NativeEditor { path: current } = tab
        {
            focused_tabs.push(current.clone());
        }
    }
    assert_eq!(copies, 1);
    assert_eq!(focused_tabs, vec![other]);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn plain_close_keeps_twin_tab_copies() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/twin.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    // Same file open in two top-level tabs of one project.
    let editor = || Tab::NativeEditor { path: path.clone() };
    let mut workspace = Workspace::from_layout(DockState::new(vec![editor()]));
    let first = workspace.tabs[0].id.clone();
    workspace.tabs.push(crate::workspace::WorkspaceTab {
        id: "second".into(),
        name: None,
        pinned: false,
        primary: Some(editor()),
        layout: DockState::new(vec![editor()]),
    });
    workspace.active = "second".into();
    app.layouts.insert("a".into(), workspace);
    // `:q` in the second tab removes only that copy: the first tab
    // keeps the file, and the buffer survives.
    app.close_native_tab(&path, false, docked_issuer("a", "second", None, false));
    assert_eq!(tab_native_paths(&app, "a", &first), vec![path.clone()]);
    assert!(tab_native_paths(&app, "a", "second").is_empty());
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn plain_close_prefers_recorded_pane_over_current_focus() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/recorded.md");
    let other = PathBuf::from("/a/other.md");
    for file in [&path, &other] {
        app.native_docs.insert(
            file.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(file.clone()),
        );
    }
    // Left leaf holds only the file; the right leaf holds the file
    // plus another tab. Focus sits left, but the recorded pane is
    // right (focus moved after the command started).
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    let [left, right] = workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![
            Tab::NativeEditor {
                path: other.clone(),
            },
            Tab::NativeEditor { path: path.clone() },
        ],
    );
    workspace.tabs[0]
        .layout
        .set_focused_node_and_surface(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: left,
        });
    app.layouts.insert("a".into(), workspace);
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    let recorded = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: right,
    };
    app.close_native_tab(
        &path,
        false,
        docked_issuer("a", &tab, Some(recorded), false),
    );
    // The recorded right leaf loses the file and keeps the other tab;
    // the focused left leaf is untouched.
    let workspace = app.layouts.get("a").expect("workspace");
    let mut right_tabs = Vec::new();
    let mut copies = 0;
    for (at, tab) in workspace.tabs[0].layout.iter_all_tabs() {
        if let Tab::NativeEditor { path: current } = tab {
            if current.as_path() == path.as_path() {
                copies += 1;
            }
            if at.node == right {
                right_tabs.push(current.clone());
            }
        }
    }
    assert_eq!(copies, 1);
    assert_eq!(right_tabs, vec![other]);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn delayed_wq_with_live_pane_closes_only_it() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/delayed.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    // Same file in two splits; the delayed `:wq` names the right
    // pane (recorded when the save started).
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    let [_, right] = workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::NativeEditor { path: path.clone() }],
    );
    app.layouts.insert("a".into(), workspace);
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    let recorded = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: right,
    };
    app.close_native_tab(&path, false, docked_issuer("a", &tab, Some(recorded), true));
    assert_eq!(docked_copies(&app, "a", &path), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn stale_delayed_wq_closes_nothing() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/stale-delayed.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    // Same file in two splits; the issuing view was closed while
    // the `:wq` save settled, so its recorded pane is stale.
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::NativeEditor { path: path.clone() }],
    );
    app.layouts.insert("a".into(), workspace);
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    let stale = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: egui_dock::NodeIndex(999),
    };
    app.close_native_tab(&path, false, docked_issuer("a", &tab, Some(stale), true));
    // No fallback may take a copy that never asked to close: both
    // stay open with the buffer.
    assert_eq!(docked_copies(&app, "a", &path), 2);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn dock_reports_each_rendered_pane_to_viewer() {
    let (mut app, ctx, _dir) = fixture();
    let path = PathBuf::from("/a/rendered.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::new(path.clone(), false),
    );
    // Same file in two splits; dock focus sits on the left leaf
    // while both panes render.
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: path.clone(),
    }]));
    let [left, right] = workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::NativeEditor { path: path.clone() }],
    );
    let main = egui_dock::SurfaceIndex::main();
    workspace.tabs[0]
        .layout
        .set_focused_node_and_surface(egui_dock::NodePath {
            surface: main,
            node: left,
        });
    let tab = workspace.tabs[0].id.clone();
    let mut viewer = crate::workspace_ui::Viewer {
        app: &mut app,
        strip: false,
        project: Some("a".into()),
        tab: Some(tab),
        window: None,
        render_path: None,
        render_tab: None,
    };
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 480.0),
            )),
            ..Default::default()
        },
        |ui| egui_dock::DockArea::new(&mut workspace).show_inside(ui, &mut viewer),
    );
    output.textures_delta.clear();
    // Each tab body reports its own leaf before drawing, so the
    // viewer holds an actually rendered pane — never dock focus
    // sampled up front (one fixed value for both panes here).
    let leaves = [
        egui_dock::NodePath {
            surface: main,
            node: left,
        },
        egui_dock::NodePath {
            surface: main,
            node: right,
        },
    ];
    assert!(
        viewer
            .render_path
            .is_some_and(|seen| leaves.contains(&seen)),
        "viewer records an actually rendered pane"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn wq_close_uses_view_issuing_the_save() {
    use crate::native_editor::CloseIssuer;
    let (mut app, ctx, _dir) = fixture();
    let path = PathBuf::from("/a/wq-issuer.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::new(path.clone(), false),
    );
    let float = egui::ViewportId::from_hash_of("wq-float");
    app.floating.push(float_window(
        "wq-float",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }])),
    );
    // `:wq` typed in the floating window: the save settles later.
    app.native_close_after_save = Some(path.clone());
    app.native_close_after_save_issuer = Some(CloseIssuer::Floating {
        viewport: float,
        node: None,
        delayed: true,
    });
    // A docked view renders first. The queued close must still target
    // the issuing floating pane — not the rendering docked copy.
    combined_frame(&mut app, &ctx, vec![]);
    // A second frame before the drain must not queue a second close
    // for the same request: it is consumed when first queued.
    combined_frame(&mut app, &ctx, vec![]);
    assert_eq!(
        app.pending_native_close.len(),
        1,
        "one close per :wq, not one per rendering copy"
    );
    app.drain_pending_native_close();
    assert!(app.floating.is_empty());
    assert_eq!(docked_copies(&app, "a", &path), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn pending_closes_drain_with_no_project_selected() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = None;
    let path = PathBuf::from("/a/hidden-float.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.floating.push(float_window(
        "hidden-float",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    // Hiding the last project must not stall floating closes: with
    // nothing checked out, the drain still runs.
    app.pending_native_close.push((path.clone(), true, None));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 480.0),
            )),
            ..Default::default()
        },
        |ui| app.window_header_tabs(ui),
    );
    output.textures_delta.clear();
    assert!(app.pending_native_close.is_empty());
    assert!(app.floating.is_empty());
    assert!(!app.native_docs.contains_key(&path));
}

#[test]
fn deferred_goto_jump_opens_cross_file_target() {
    use terminator_native_edit::lsp::{GotoTarget, Position, Range};
    let (mut app, _, _dir) = fixture();
    let from = crate::test_path("/a/from.rs");
    let to = crate::test_path("/a/to.rs");
    let jump = || crate::lsp_manager::GotoJump {
        from: from.clone(),
        targets: vec![GotoTarget {
            uri: url::Url::from_file_path(&to).unwrap().into(),
            range: Range {
                start: Position {
                    line: 4,
                    character: 0,
                },
                end: Position {
                    line: 4,
                    character: 1,
                },
            },
        }],
    };
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: from.clone(),
        }])),
    );
    // One frame: the workspace is checked out for rendering, so the
    // finished jump queues instead of resolving against the missing
    // project (which dropped it before).
    let dock = app.layouts.remove("a").expect("checked out");
    app.pending_goto.push(jump());
    app.layouts.insert("a".into(), dock);
    // Deferred work runs with the workspace checked back in: the
    // jump resolves and opens the target.
    app.drain_pending_native_close();
    assert_eq!(app.native_pending_line.get(&to), Some(&4));
    let absolute = std::path::absolute(&to).unwrap();
    assert_eq!(app.native_pending_col.get(&absolute), Some(&0));
    // A jump whose project is truly gone applies nothing.
    app.layouts.remove("a");
    app.pending_goto.push(jump());
    app.drain_pending_native_close();
    assert_eq!(app.native_pending_line.len(), 1);
}

#[test]
fn async_close_completion_keeps_a_buffer_that_became_dirty() {
    let (mut app, _ctx, dir) = fixture();
    let path = dir.path().join("late.rs");
    std::fs::write(&path, "source").unwrap();
    let tab = Tab::NativeEditor { path: path.clone() };
    let workspace = Workspace::from_layout(DockState::new(vec![tab]));
    let tab_id = workspace.tabs[0].id.clone();
    app.layouts.insert("a".into(), workspace);
    // The editor check started while the buffer was clean, then the user
    // typed while the GUI stayed usable.
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.close_workspace = Some(("a".into(), tab_id.clone()));
    app.close_workspace_tab_now("a", &tab_id);
    assert!(app.layouts["a"].tabs.iter().any(|tab| tab.id == tab_id));
    assert!(app.native_dirty(&path));
    assert!(matches!(&app.native_close_prompt, Some((p, None)) if *p == path));
    assert!(app.close_workspace.is_none());
}
