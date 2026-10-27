use super::*;

#[cfg(feature = "test-support")]
#[test]
fn single_native_editor_panes_have_working_dock_drag_handles() {
    let (mut app, ctx, dir) = fixture();
    let left = dir.path().join("left.rs");
    let right = dir.path().join("right.rs");
    for path in [&left, &right] {
        std::fs::write(path, "source").unwrap();
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
        );
    }
    let from = Tab::NativeEditor { path: left };
    let to = Tab::NativeEditor { path: right };
    let mut workspace = Workspace::from_layout(DockState::new(vec![from.clone()]));
    workspace
        .main_surface_mut()
        .split_right(egui_dock::NodeIndex::root(), 0.5, vec![to.clone()]);
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    combined_frame(&mut app, &ctx, vec![]);
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, &format!("leaf-tab:{}", from.key()));
    let target = app.layouts["a"].find_tab(&to).unwrap().node_path();
    let end = app.layouts["a"].leaf(target).unwrap().rect.center();
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, end, 8);
    combined_frame(&mut app, &ctx, vec![frame_press(end, false)]);
    combined_frame(&mut app, &ctx, vec![]);
    let dock = &app.layouts["a"];
    assert_eq!(
        dock.find_tab(&from).unwrap().node_path(),
        dock.find_tab(&to).unwrap().node_path()
    );
    assert_eq!(dock.iter_all_tabs().count(), 2);
    assert!(app.native_dirty(match &from {
        Tab::NativeEditor { path } => path,
        _ => unreachable!(),
    }));
}

#[test]
fn drag_ghost_paints_without_changing_state() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    app.drag.pane = Some(Tab::Terminal("one".into()));
    let pos = egui::pos2(500.0, 300.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |ui| {
            app.paint_drag_ghost(ui);
        },
    );
    output.textures_delta.clear();
    assert_eq!(app.drag.pane, Some(Tab::Terminal("one".into())));
}

/// Hovering another strip tab mid-drag previews its splits; dropping on
/// one of its leaves lands the terminal precisely there.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_preview_switches_tab_and_drops_into_its_leaf() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two", "three"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("three".into())],
    );
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-drag:one");
    let dest = frame_center(&app, &ctx, "workspace-tab:two");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, dest, 4);
    // The destination tab content is previewed while hovering its strip
    // tab, remembering the origin tab.
    assert_eq!(app.layouts["a"].active, "tB");
    assert_eq!(
        app.drag.preview_origin,
        Some(("a".to_owned(), group_a.clone()))
    );
    let leaf_caption = frame_center(&app, &ctx, "pane-drag:three");
    // Aim at the middle of the previewed split: its caption sits in the
    // top edge band, which would split instead of swapping.
    let leaf = egui::pos2(leaf_caption.x, 320.0);
    frame_glide(&mut app, &ctx, dest, leaf, 3);
    combined_frame(&mut app, &ctx, vec![frame_press(leaf, false)]);
    assert!(app.drag.pane.is_none());
    assert!(app.drag.preview_origin.is_none());
    let dock = app.layouts.get("a").unwrap();
    // Single panes swap: both tabs survive with everything visible.
    assert_eq!(dock.tabs.len(), 2);
    assert_eq!(dock.active, "tB");
    // "one" landed in the targeted previewed leaf.
    let landed = dock
        .find_tab(&Tab::Terminal("one".into()))
        .unwrap()
        .node_path();
    assert!(
        dock.leaf(landed).unwrap().rect.contains(leaf),
        "drop missed the targeted leaf"
    );
    // "three" swapped back into the origin tab.
    let origin = dock
        .tabs
        .iter()
        .find(|tab| tab.id != "tB")
        .expect("origin tab survives the swap");
    assert!(
        origin
            .layout
            .find_tab(&Tab::Terminal("three".into()))
            .is_some()
    );
}

/// Dropping a pane near the edge of another split opens it in a new
/// split beside that leaf instead of swapping.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_edge_drop_opens_a_split() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["left", "right"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("left".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-drag:left");
    let right = app
        .fixture_rect(&ctx, "pane-drag:right")
        .expect("caption geometry");
    // Near the right edge of the right split, vertically centered on
    // its caption so the top band cannot win the zone.
    let edge = egui::pos2(right[0] + right[2] - 4.0, right[1] + right[3] / 2.0);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, edge, 4);
    assert_eq!(
        app.drag.pane,
        Some(Tab::Terminal("left".into())),
        "caption drag did not start"
    );
    combined_frame(&mut app, &ctx, vec![frame_press(edge, false)]);
    assert!(app.drag.pane.is_none());
    // One more frame so leaf rectangles reflect the new split.
    combined_frame(&mut app, &ctx, vec![]);
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(dock.iter_leaves().count(), 2);
    let left_rect = dock
        .find_tab(&Tab::Terminal("left".into()))
        .map(|path| dock.leaf(path.node_path()).unwrap().rect)
        .unwrap();
    let right_rect = dock
        .find_tab(&Tab::Terminal("right".into()))
        .map(|path| dock.leaf(path.node_path()).unwrap().rect)
        .unwrap();
    assert!(
        left_rect.center().x > right_rect.center().x,
        "edge drop did not split right"
    );
    assert_eq!(dock.active_pane(), Some(&Tab::Terminal("left".into())));
}

/// Dragging a lower-pane tab up into the main dock lands that shell there.
/// IDE mode stays on and the editor stays.
#[cfg(feature = "test-support")]
#[test]
fn strip_tab_drag_lands_in_the_main_pane() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("low", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("low".into())]),
    );
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
            "edit".into(),
        )])),
    );
    cross_dock_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "strip-tab:low");
    let caption = frame_center(&app, &ctx, "pane-drag:edit");
    let strip = app
        .fixture_rect(&ctx, "ide-terminal-strip")
        .expect("strip geometry");
    let drop_at = egui::pos2(caption.x, (caption.y + strip[1]) * 0.5);
    cross_dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    cross_dock_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    cross_glide(&mut app, &ctx, start, drop_at, 8);
    assert!(
        app.drag.pane_from_strip,
        "leaving the strip did not promote the tab drag"
    );
    cross_dock_frame(&mut app, &ctx, vec![frame_press(drop_at, false)]);
    assert!(app.drag.pane.is_none());
    assert!(app.preferences.ide_mode);
    assert!(!app.is_strip_session("a", "low"));
    let dock = app.layouts.get("a").unwrap();
    assert!(dock.contains(&Tab::Terminal("low".into())));
    assert!(dock.contains(&Tab::Terminal("edit".into())));
}

/// Dragging a main-pane shell down onto the IDE strip moves only that shell.
#[cfg(feature = "test-support")]
#[test]
fn main_caption_drag_lands_in_the_strip() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.preferences.ide_terminal_collapsed = true;
    app.state.sessions = vec![
        session_fixture("up", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    let mut main = egui_dock::DockState::new(vec![Tab::Terminal("edit".into())]);
    main.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("up".into())],
    );
    app.layouts.insert("a".into(), Workspace::from_layout(main));
    cross_dock_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-drag:up");
    let strip = app
        .fixture_rect(&ctx, "ide-terminal-strip")
        .expect("strip geometry");
    let drop_at = egui::pos2(strip[0] + strip[2] * 0.5, strip[1] + strip[3] * 0.5);
    cross_dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    cross_dock_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    cross_glide(&mut app, &ctx, start, drop_at, 8);
    assert_eq!(app.drag.pane, Some(Tab::Terminal("up".into())));
    assert!(!app.drag.pane_from_strip);
    cross_dock_frame(&mut app, &ctx, vec![frame_press(drop_at, false)]);
    assert!(app.drag.pane.is_none());
    assert!(app.preferences.ide_mode);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert!(app.is_strip_session("a", "up"));
    assert!(!app.is_strip_session("a", "edit"));
    let dock = app.layouts.get("a").unwrap();
    assert!(!dock.contains(&Tab::Terminal("up".into())));
    assert!(dock.contains(&Tab::Terminal("edit".into())));
}

/// Cancelling a pane drag (Esc) after previewing another tab switches
/// back to the origin tab without moving anything.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_cancel_restores_origin_tab() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-drag:one");
    let dest = frame_center(&app, &ctx, "workspace-tab:two");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
    assert_eq!(app.layouts["a"].active, "tB");
    combined_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }],
    );
    assert!(app.drag.pane.is_none());
    assert!(app.drag.preview_origin.is_none());
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(dock.active, group_a);
    assert_eq!(dock.tabs.len(), 2);
    assert!(dock.contains(&Tab::Terminal("one".into())));
}

/// Dropping a dragged pane into a gap between strip tabs opens it in a
/// fresh top-level tab at that slot instead of appending at the end.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_gap_drop_creates_tab_at_slot() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two", "three"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("three".into())],
    );
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    combined_frame(&mut app, &ctx, vec![]);
    let tab_a = app
        .fixture_rect(&ctx, "workspace-tab:one")
        .expect("tab geometry");
    let tab_b = app
        .fixture_rect(&ctx, "workspace-tab:two")
        .expect("tab geometry");
    let (left, right) = if tab_a[0] < tab_b[0] {
        (tab_a, tab_b)
    } else {
        (tab_b, tab_a)
    };
    let gap = egui::pos2(
        (left[0] + left[2] + right[0]) / 2.0,
        left[1] + left[3] / 2.0,
    );
    let start = frame_center(&app, &ctx, "pane-drag:one");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, gap, 4);
    combined_frame(&mut app, &ctx, vec![frame_press(gap, false)]);
    assert!(app.drag.pane.is_none());
    assert!(app.drag.preview_origin.is_none());
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(dock.tabs.len(), 3);
    assert!(
        dock.tabs[0]
            .layout
            .find_tab(&Tab::Terminal("three".into()))
            .is_some()
    );
    assert!(
        dock.tabs[1]
            .layout
            .find_tab(&Tab::Terminal("one".into()))
            .is_some()
    );
    assert!(
        dock.tabs[2]
            .layout
            .find_tab(&Tab::Terminal("two".into()))
            .is_some()
    );
    assert_eq!(dock.active, dock.tabs[1].id);
}

/// Dragging a strip tab reorders the top-level tabs to the drop slot.
#[cfg(feature = "test-support")]
#[test]
fn strip_tab_drag_reorders_top_level_tabs() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    let group_a = workspace.tabs[0].id.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("two".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "workspace-tab:two");
    let first = app
        .fixture_rect(&ctx, "workspace-tab:one")
        .expect("tab geometry");
    // Inside the first tab's left edge band: a gap for insertion math
    // while still on the strip for the drop.
    let dest = egui::pos2(first[0] + 5.0, first[1] + first[3] / 2.0);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, dest, 4);
    assert_eq!(app.drag.tab, Some("tB".to_owned()));
    combined_frame(&mut app, &ctx, vec![frame_press(dest, false)]);
    assert!(app.drag.tab.is_none());
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(dock.ids(), vec!["tB".to_owned(), group_a.clone()]);
    assert_eq!(dock.active, "tB");
}

/// Hovering a strip tab mid pane-drag washes the previewed tab's
/// focused leaf at real size, showing where a release would land.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_strip_hover_washes_the_landing_leaf() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two", "three"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("three".into())],
    );
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-drag:one");
    let dest = frame_center(&app, &ctx, "workspace-tab:two");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, dest, 4);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
    // Still dragging: the destination tab is previewed, nothing moved.
    assert_eq!(app.drag.pane, Some(Tab::Terminal("one".into())));
    assert_eq!(app.layouts["a"].active, "tB");
    let wash = app
        .fixture_rect(&ctx, "strip-drop-wash")
        .expect("landing wash");
    assert!(
        wash[2] > 200.0 && wash[3] > 100.0,
        "wash must cover the landing leaf at real size, got {wash:?}"
    );
}

/// Dragging a pane caption onto the strip + opens it in a fresh
/// top-level tab (the GUI drag-detach flow: press-hold on the
/// caption, glide to the strip +, release).
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_onto_strip_plus_opens_new_tab() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("two".into())],
    );
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    combined_frame(&mut app, &ctx, vec![]);
    let start = frame_center(&app, &ctx, "pane-caption:one");
    let dest = frame_center(&app, &ctx, "workspace-plus");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, dest, 4);
    assert_eq!(app.drag.pane, Some(Tab::Terminal("one".into())));
    combined_frame(&mut app, &ctx, vec![frame_press(dest, false)]);
    assert!(app.drag.pane.is_none());
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(dock.tabs.len(), 2, "drop missed the strip +");
    assert!(
        dock.tabs
            .iter()
            .any(|tab| tab.layout.find_tab(&Tab::Terminal("one".into())).is_some())
            && dock
                .tabs
                .iter()
                .any(|tab| tab.layout.find_tab(&Tab::Terminal("two".into())).is_some()),
        "panes lost in the detach"
    );
}

/// A lone terminal pane offers a caption `+` that stacks a tab in its
/// own split: the same leaf-anchored request as the leaf tab bar `+`,
/// which a lone pane hides.
#[cfg(feature = "test-support")]
#[test]
fn caption_stack_button_queues_in_split_tab() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())])),
    );
    app.selected = Some("a".into());
    combined_frame(&mut app, &ctx, vec![]);
    let at = frame_center(&app, &ctx, "pane-stack:one");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    combined_frame(&mut app, &ctx, vec![frame_press(at, true)]);
    combined_frame(&mut app, &ctx, vec![frame_press(at, false)]);
    let path = app.layouts["a"]
        .find_tab(&Tab::Terminal("one".into()))
        .expect("pane placed")
        .node_path();
    assert_eq!(app.add_tab, Some((path, None)));
}

/// A tab reorder drag paints a tab-sized ghost that tracks the pointer.
#[cfg(feature = "test-support")]
#[test]
fn tab_drag_ghost_follows_the_pointer() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.drag.tab = Some("tB".to_owned());
    let pos = egui::pos2(500.0, 300.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |ui| {
            app.paint_tab_ghost(ui);
        },
    );
    output.textures_delta.clear();
    let ghost = app.fixture_rect(&ctx, "tab-ghost").expect("tab ghost");
    assert!(
        ghost[2] < 180.0 && ghost[2] > 60.0,
        "short tab ghost must hug the title, got {ghost:?}"
    );
    assert_eq!(ghost[3], 32.0);
    assert!(
        (ghost[0] - (pos.x - ghost[2] / 2.0)).abs() < 2.0
            && (ghost[1] - (pos.y - 16.0)).abs() < 2.0,
        "ghost must track the pointer, got {ghost:?}"
    );
    assert_eq!(app.drag.tab, Some("tB".to_owned()));
}

/// A pane dragged over a strip gap shows the tab-sized ghost (the
/// new-tab outcome), never stacked with the pane snapshot ghost.
#[cfg(feature = "test-support")]
#[test]
fn pane_drag_strip_gap_shows_tab_ghost() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two", "three"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("three".into())],
    );
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    combined_frame(&mut app, &ctx, vec![]);
    let tab_a = app
        .fixture_rect(&ctx, "workspace-tab:one")
        .expect("tab geometry");
    let tab_b = app
        .fixture_rect(&ctx, "workspace-tab:two")
        .expect("tab geometry");
    let (left, right) = if tab_a[0] < tab_b[0] {
        (tab_a, tab_b)
    } else {
        (tab_b, tab_a)
    };
    let gap = egui::pos2(
        (left[0] + left[2] + right[0]) / 2.0,
        left[1] + left[3] / 2.0,
    );
    let start = frame_center(&app, &ctx, "pane-drag:one");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
    frame_glide(&mut app, &ctx, start, gap, 4);
    assert!(!app.drag.strip_tab_hover);
    assert!(app.drag.strip_new_tab_hover);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events: vec![egui::Event::PointerMoved(gap)],
            ..Default::default()
        },
        |ui| {
            app.paint_drag_ghost(ui);
            app.paint_tab_ghost(ui);
        },
    );
    output.textures_delta.clear();
    let ghost = app.fixture_rect(&ctx, "tab-ghost").expect("tab ghost");
    assert!(
        ghost[2] < 180.0 && ghost[2] > 60.0,
        "short tab ghost must hug the title, got {ghost:?}"
    );
    assert_eq!(ghost[3], 32.0);
    assert!(
        (ghost[0] - (gap.x - ghost[2] / 2.0)).abs() < 2.0,
        "tab ghost must track the pointer, got {ghost:?}"
    );
    assert!(
        app.fixture_rect(&ctx, "pane-ghost").is_none(),
        "pane ghost must yield to the tab ghost over the strip"
    );
    assert_eq!(app.drag.pane, Some(Tab::Terminal("one".into())));
}

/// Dragging a terminal caption onto another split leaf rearranges the
/// active tab (single panes swap). Runs headless with synthetic pointer
/// events; needs test-support for the caption geometry records.
#[cfg(feature = "test-support")]
#[test]
fn pane_caption_drag_swaps_split_terminals() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("left", SessionKind::Shell));
    app.state
        .sessions
        .push(session_fixture("right", SessionKind::Shell));
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    let left_home = workspace
        .find_tab(&Tab::Terminal("left".into()))
        .unwrap()
        .node_path();
    let right_home = workspace
        .find_tab(&Tab::Terminal("right".into()))
        .unwrap()
        .node_path();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("left".into());
    fn dock_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                app.paint_dock(ui, "a", &mut dock);
                app.layouts.insert("a".into(), dock);
            },
        );
        output.textures_delta.clear();
    }
    fn center(rect: [f32; 4]) -> egui::Pos2 {
        egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)
    }
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    dock_frame(&mut app, &ctx, vec![]);
    let start = center(
        app.fixture_rect(&ctx, "pane-drag:left")
            .expect("left caption geometry"),
    );
    let end = center(
        app.fixture_rect(&ctx, "pane-drag:right")
            .expect("right caption geometry"),
    );
    // Aim at the middle of the right split: the caption sits in the top
    // edge band, which would split instead of swapping.
    let middle = egui::pos2(end.x, 300.0);
    dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    dock_frame(&mut app, &ctx, vec![press(start, true)]);
    assert!(app.drag.pane.is_none());
    for step in 1..=4 {
        let k = f32::from(i16::try_from(step).unwrap_or(0)) / 4.0;
        dock_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                start.x + (middle.x - start.x) * k,
                start.y + (middle.y - start.y) * k,
            ))],
        );
    }
    assert_eq!(
        app.drag.pane,
        Some(Tab::Terminal("left".into())),
        "caption drag did not start"
    );
    dock_frame(&mut app, &ctx, vec![press(middle, false)]);
    assert!(app.drag.pane.is_none());
    // A middle drop swaps the two single panes in place: same leaves,
    // exchanged terminals.
    let dock = app.layouts.get("a").unwrap();
    assert_eq!(
        dock.find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path(),
        right_home,
        "middle drop did not swap into the right split"
    );
    assert_eq!(
        dock.find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path(),
        left_home,
        "middle drop did not swap into the left split"
    );
}

/// Dragging a terminal caption onto another workspace strip tab moves it
/// across top-level tabs. Needs test-support for geometry records.
#[cfg(feature = "test-support")]
#[test]
fn pane_caption_drop_on_strip_tab_moves_across_groups() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    app.state
        .sessions
        .push(session_fixture("two", SessionKind::Shell));
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    let group_a = workspace.tabs[0].id.clone();
    workspace.active = group_a.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    fn combined_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                // Same order as the real panels: strip first, dock after.
                let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                app.workspace_bar(ui, "a", &mut dock);
                app.paint_dock(ui, "a", &mut dock);
                app.layouts.insert("a".into(), dock);
            },
        );
        output.textures_delta.clear();
    }
    fn center(rect: [f32; 4]) -> egui::Pos2 {
        egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)
    }
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    combined_frame(&mut app, &ctx, vec![]);
    let start = center(
        app.fixture_rect(&ctx, "pane-drag:one")
            .expect("caption geometry"),
    );
    let dest = center(
        app.fixture_rect(&ctx, "workspace-tab:two")
            .expect("strip tab geometry"),
    );
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
    combined_frame(&mut app, &ctx, vec![press(start, true)]);
    for step in 1..=4 {
        let k = f32::from(i16::try_from(step).unwrap_or(0)) / 4.0;
        combined_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                start.x + (dest.x - start.x) * k,
                start.y + (dest.y - start.y) * k,
            ))],
        );
    }
    assert_eq!(
        app.drag.pane,
        Some(Tab::Terminal("one".into())),
        "caption drag did not start"
    );
    combined_frame(&mut app, &ctx, vec![press(dest, false)]);
    assert!(app.drag.pane.is_none());
    let dock = app.layouts.get("a").unwrap();
    // Both sides held one terminal: they swap instead of hiding one.
    assert_eq!(dock.tabs.len(), 2, "swap must keep both tabs");
    assert_eq!(dock.active, "tB");
    assert!(dock.contains(&Tab::Terminal("one".into())));
    assert!(dock.contains(&Tab::Terminal("two".into())));
}
