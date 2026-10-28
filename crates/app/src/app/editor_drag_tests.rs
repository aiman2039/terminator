use super::super::*;
use super::tests::support::*;
use egui_dock::{DockState, NodeIndex, NodePath, SurfaceIndex, TabIndex, TabInsert};

fn fixture_with_copies() -> (App, egui::Context, tempfile::TempDir, Tab, PaneSource) {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("editor.rs");
    let target = dir.path().join("target.rs");
    for file in [&path, &target] {
        std::fs::write(file, "source").unwrap();
        app.native_docs.insert(
            file.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(file.clone()),
        );
    }
    let pane = Tab::NativeEditor { path };
    let mut dock = Workspace::from_layout(DockState::new(vec![pane.clone()]));
    let group = dock.active.clone();
    let [_, right] =
        dock.main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![pane.clone()]);
    let source = PaneSource {
        project: "a".into(),
        group: group.clone(),
        path: (
            NodePath {
                surface: SurfaceIndex::main(),
                node: right,
            },
            TabIndex(0),
        )
            .into(),
    };
    dock.add("destination".into(), Tab::NativeEditor { path: target });
    dock.active = group;
    app.layouts.insert("a".into(), dock);
    app.selected = Some("a".into());
    (app, ctx, dir, pane, source)
}

#[cfg(feature = "test-support")]
fn begin_drag(app: &mut App, ctx: &egui::Context, source: &PaneSource) -> egui::Pos2 {
    combined_frame(app, ctx, vec![]);
    combined_frame(app, ctx, vec![]);
    let name = format!(
        "leaf-tab-at:{}:{}:{}",
        source.path.surface.0, source.path.node.0, source.path.tab.0
    );
    let start = frame_center(app, ctx, &name);
    combined_frame(app, ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(app, ctx, vec![frame_press(start, true)]);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Bounded fixture coordinates."
    )]
    let moved = start + egui::vec2(18.0, 12.0);
    frame_glide(FrameGlideInput {
        app,
        ctx,
        from: start,
        to: moved,
        steps: 3,
    });
    assert_eq!(app.drag.pane_source.as_ref(), Some(source));
    start
}

#[test]
#[cfg(feature = "test-support")]
fn editor_drag_moves_the_selected_same_file_copy_to_another_workspace() {
    let (mut app, ctx, _dir, pane, source) = fixture_with_copies();
    let start = begin_drag(&mut app, &ctx, &source);
    let end = frame_center(&app, &ctx, "workspace-tab:target.rs");
    frame_glide(FrameGlideInput {
        app: &mut app,
        ctx: &ctx,
        from: start,
        to: end,
        steps: 8,
    });
    combined_frame(&mut app, &ctx, vec![frame_press(end, false)]);
    combined_frame(&mut app, &ctx, vec![]);
    let dock = &app.layouts["a"];
    let origin = dock.tabs.iter().find(|tab| tab.id == source.group).unwrap();
    assert_eq!(origin.layout.iter_all_tabs().count(), 1);
    assert_eq!(origin.layout.iter_all_tabs().next().unwrap().1, &pane);
    let destination = dock
        .tabs
        .iter()
        .find(|tab| tab.id == "destination")
        .unwrap();
    assert_eq!(destination.layout.iter_all_tabs().count(), 2);
    assert!(destination.layout.find_tab(&pane).is_some());
    assert_eq!(dock.active, "destination");
    assert!(app.drag.pane.is_none());
    assert!(app.active_session.is_none());
    assert!(app.native_dirty(match &pane {
        Tab::NativeEditor { path } => path,
        _ => unreachable!(),
    }));
    let restored = Workspace::load(serde_json::to_value(dock).unwrap()).unwrap();
    assert_eq!(
        restored
            .tabs
            .iter()
            .map(|tab| tab.layout.iter_all_tabs().count())
            .sum::<usize>(),
        3
    );
}

#[test]
#[cfg(feature = "test-support")]
fn editor_drag_to_plus_and_a_strip_gap_creates_one_new_workspace() {
    for plus in [true, false] {
        let (mut app, ctx, _dir, pane, source) = fixture_with_copies();
        let start = begin_drag(&mut app, &ctx, &source);
        let end = if plus {
            frame_center(&app, &ctx, "workspace-plus")
        } else {
            let first = app.fixture_rect(&ctx, "workspace-tab:editor.rs").unwrap();
            let second = app.fixture_rect(&ctx, "workspace-tab:target.rs").unwrap();
            egui::pos2(
                (first[0] + first[2] + second[0]) / 2.0,
                first[1] + first[3] / 2.0,
            )
        };
        frame_glide(FrameGlideInput {
            app: &mut app,
            ctx: &ctx,
            from: start,
            to: end,
            steps: 8,
        });
        combined_frame(&mut app, &ctx, vec![frame_press(end, false)]);
        combined_frame(&mut app, &ctx, vec![]);
        let dock = &app.layouts["a"];
        assert_eq!(dock.tabs.len(), 3);
        assert_eq!(dock.iter_all_tabs().count(), 1);
        assert_eq!(dock.iter_all_tabs().next().unwrap().1, &pane);
        assert_eq!(
            dock.tabs
                .iter()
                .map(|tab| tab.layout.iter_all_tabs().count())
                .sum::<usize>(),
            3
        );
        assert!(app.drag.pane.is_none());
    }
}

#[test]
#[cfg(feature = "test-support")]
fn escape_cancels_editor_drag_and_restores_origin_workspace() {
    let (mut app, ctx, _dir, _pane, source) = fixture_with_copies();
    let start = begin_drag(&mut app, &ctx, &source);
    let end = frame_center(&app, &ctx, "workspace-tab:target.rs");
    frame_glide(FrameGlideInput {
        app: &mut app,
        ctx: &ctx,
        from: start,
        to: end,
        steps: 8,
    });
    assert_eq!(app.layouts["a"].active, "destination");
    combined_frame(&mut app, &ctx, vec![key_press(egui::Key::Escape)]);
    combined_frame(&mut app, &ctx, vec![frame_press(end, false)]);
    assert_eq!(app.layouts["a"].active, source.group);
    assert_eq!(app.layouts["a"].tabs.len(), 2);
    assert!(app.drag.pane.is_none());
    assert!(app.drag.pane_source.is_none());
}

#[test]
fn exact_editor_moves_reject_stale_sources_and_keep_stack_order() {
    let (mut app, _, _dir, pane, source) = fixture_with_copies();
    let dock = app.layouts.get_mut("a").unwrap();
    let mut stale = source.clone();
    stale.path.tab = TabIndex(4);
    let before = serde_json::to_value(&*dock).unwrap();
    assert!(!dock.move_view_to_group(&stale, &pane, "destination"));
    assert_eq!(serde_json::to_value(&*dock).unwrap(), before);
    let other = Tab::NativeEditor {
        path: "other.rs".into(),
    };
    let mut dock = Workspace::from_layout(DockState::new(vec![
        pane.clone(),
        other.clone(),
        pane.clone(),
    ]));
    let source = PaneSource {
        project: "a".into(),
        group: dock.active.clone(),
        path: (
            NodePath {
                surface: SurfaceIndex::main(),
                node: NodeIndex::root(),
            },
            TabIndex(2),
        )
            .into(),
    };
    assert!(
        dock.move_view_to_leaf(crate::workspace::MoveViewToLeafInput {
            source: &source,
            pane: &pane,
            group: &source.group,
            dest: source.path.node_path(),
            insertion: TabInsert::Insert(TabIndex(0))
        })
    );
    let tabs = &dock.leaf(source.path.node_path()).unwrap().tabs;
    assert_eq!(tabs, &[pane.clone(), pane, other]);
}

#[test]
fn detach_and_float_use_the_selected_editor_copy_and_unique_windows() {
    let (mut app, _ctx, _dir, pane, source) = fixture_with_copies();
    let mut dock = app.layouts.remove("a").unwrap();
    app.detach_pane = Some(pane.clone());
    app.pane_action_source = Some(source.clone());
    app.apply_detach_pane(&mut dock);
    assert_eq!(
        dock.tabs
            .iter()
            .find(|tab| tab.id == source.group)
            .unwrap()
            .layout
            .iter_all_tabs()
            .count(),
        1
    );
    assert_eq!(dock.tabs.len(), 3);
    for _ in 0..2 {
        let group = dock
            .tabs
            .iter()
            .find(|tab| tab.layout.find_tab(&pane).is_some())
            .unwrap();
        let source = PaneSource {
            project: "a".into(),
            group: group.id.clone(),
            path: group.layout.find_tab(&pane).unwrap(),
        };
        app.float_pane = Some(pane.clone());
        app.pane_action_source = Some(source);
        app.apply_float_pane("a", &mut dock);
    }
    assert_eq!(app.floating.len(), 2);
    assert_ne!(app.floating[0].viewport, app.floating[1].viewport);
    app.layouts.insert("a".into(), dock);
    app.dock_back_all_floating();
    assert!(app.floating.is_empty());
    assert_eq!(
        app.layouts["a"]
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.iter_all_tabs())
            .filter(|(_, tab)| **tab == pane)
            .count(),
        2
    );
}
