use super::*;

#[test]
#[cfg(feature = "test-support")]
fn single_terminal_tab_paints_the_terminal_without_a_node() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions = vec![
        session_fixture("s1", SessionKind::Shell),
        session_fixture("s2", SessionKind::Shell),
    ];
    let tab = |id: &str, sid: &str| crate::workspace::WorkspaceTab {
        id: id.into(),
        name: None,
        pinned: false,
        primary: Some(Tab::Terminal(sid.into())),
        layout: egui_dock::DockState::new(vec![Tab::Terminal(sid.into())]),
    };
    let mut workspace =
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal("s1".into())]));
    workspace.tabs[0] = tab("g1", "s1");
    workspace.tabs.push(tab("g2", "s2"));
    workspace.active = "g1".into();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());

    let mut time = 0.0_f64;
    let mut paint = |app: &mut App| {
        time += 0.05;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 600.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
    };
    paint(&mut app);
    paint(&mut app);
    // Both terminals are painted at the tab level; no tab nodes.
    assert!(header_target(&ctx)("session-row:s1").is_some());
    assert!(header_target(&ctx)("session-row:s2").is_some());
    assert!(header_target(&ctx)("sidebar-tab:g1").is_none());
    assert!(header_target(&ctx)("sidebar-tab:g2").is_none());

    // A custom name restores the node, so the name stays visible.
    app.layouts.get_mut("a").unwrap().tabs[0].name = Some("Agents".into());
    paint(&mut app);
    paint(&mut app);
    assert!(header_target(&ctx)("sidebar-tab:g1").is_some());
    assert!(header_target(&ctx)("session-row:s1").is_some());
}

#[test]
fn sidebar_navigation_selects_owning_top_level_tab() {
    let (mut app, _, _dir) = fixture();
    app.state.sessions.extend([
        session_fixture("one", SessionKind::Shell),
        session_fixture("two", SessionKind::Shell),
    ]);
    app.insert("a", Tab::Terminal("one".into()), None);
    let first = app.layouts["a"].active.clone();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("second".into(), Tab::Terminal("two".into()));
    app.go_session("one");
    assert_eq!(app.layouts["a"].active, first);
    assert_eq!(app.active_session.as_deref(), Some("one"));
    app.go_session("two");
    assert_eq!(app.layouts["a"].active, "second");
    assert_eq!(app.layouts["a"].tabs.len(), 2);
}

#[test]
fn closing_either_split_direction_expands_the_remaining_pane() {
    for direction in ["left", "right", "up", "down"] {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("remaining", SessionKind::Shell),
            session_fixture("closed", SessionKind::Shell),
        ]);
        app.insert("a", Tab::Terminal("remaining".into()), None);
        app.insert("a", Tab::Terminal("closed".into()), Some(direction));
        app.active_session = Some("closed".into());
        let mut next = app.state.clone();
        next.sessions
            .iter_mut()
            .find(|s| s.id == "closed")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(next);
        let leaf = app.layouts["a"].main_surface()[NodeIndex::root()]
            .get_leaf()
            .expect("Remaining pane should replace the split root");
        assert_eq!(leaf.tabs, vec![Tab::Terminal("remaining".into())]);
        assert_eq!(app.active_session.as_deref(), Some("remaining"));
        assert!(
            app.state
                .sessions
                .iter()
                .any(|s| s.id == "closed" && s.lifecycle == Lifecycle::Ended)
        );
    }
}

/// Short titles stay narrow, long titles grow, and the active tab's
/// accent underline is painted inside the tab rather than on its edge.
#[cfg(feature = "test-support")]
#[test]
fn workspace_tabs_hug_their_labels_and_underline_inside() {
    let (mut app, ctx, _dir) = fixture();
    let mut short = session_fixture("one", SessionKind::Shell);
    short.label = "one".into();
    let mut long = session_fixture("two", SessionKind::Shell);
    long.label = "codex supervisor".into();
    app.state.sessions.push(short);
    app.state.sessions.push(long);
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    workspace.active = workspace.tabs[0].id.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    combined_frame(&mut app, &ctx, vec![]);
    let one = app
        .fixture_rect(&ctx, "workspace-tab:one")
        .expect("short tab");
    let long = app
        .fixture_rect(&ctx, "workspace-tab:codex supervisor")
        .expect("long tab");
    assert!(
        one[2] < long[2] && one[2] < 160.0 && long[2] < 220.0,
        "tabs must hug the label, got one={one:?} long={long:?}"
    );
    let underline = app
        .fixture_rect(&ctx, "workspace-tab-underline:one")
        .expect("active underline");
    let tab_bottom = one[1] + one[3];
    assert!(
        underline[1] > one[1]
            && underline[1] + underline[3] < tab_bottom
            && underline[0] >= one[0]
            && underline[0] + underline[2] <= one[0] + one[2] + 0.5,
        "underline must sit inside the tab, tab={one:?} bar={underline:?}"
    );
    assert!(
        app.fixture_rect(&ctx, "pane-caption:one").is_some(),
        "a lone pane row stays addressable for its menu"
    );
    assert!(
        app.fixture_rect(&ctx, "pane-title:one").is_none(),
        "a lone pane must not repeat the workspace tab title"
    );
}

/// Split panes keep a caption so each one can be named and dragged.
#[cfg(feature = "test-support")]
#[test]
fn split_panes_keep_their_captions() {
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
    combined_frame(&mut app, &ctx, vec![]);
    assert!(app.fixture_rect(&ctx, "pane-caption:left").is_some());
    assert!(app.fixture_rect(&ctx, "pane-caption:right").is_some());
}

/// Every terminal caption carries Git and both split buttons on one row.
#[cfg(feature = "test-support")]
#[test]
fn terminal_bars_keep_git_and_splits_on_one_row() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["left", "right"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    app.metadata = Some(metadata::Metadata {
        cwd: "/a".into(),
        branch: Some("master".into()),
        ..Default::default()
    });
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    combined_frame(&mut app, &ctx, vec![]);
    for sid in ["left", "right"] {
        let caption = app
            .fixture_rect(&ctx, &format!("pane-caption:{sid}"))
            .expect("caption");
        for (name, action) in [
            ("git", "pane-git"),
            ("vertical", "pane-split-vertical"),
            ("horizontal", "pane-split-horizontal"),
        ] {
            let rect = app
                .fixture_rect(&ctx, &format!("{action}:{sid}"))
                .unwrap_or_else(|| panic!("missing {name} on {sid}"));
            assert!(
                (rect[1] + rect[3] / 2.0 - (caption[1] + caption[3] / 2.0)).abs() < 4.0,
                "{name} on {sid} should sit on the caption row"
            );
            assert!(
                rect[0] >= caption[0] && rect[0] + rect[2] <= caption[0] + caption[2] + 1.0,
                "{name} on {sid} should stay inside the caption"
            );
        }
    }
    let git = frame_center(&app, &ctx, "pane-git:left");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(git)]);
    combined_frame(&mut app, &ctx, vec![frame_press(git, true)]);
    combined_frame(&mut app, &ctx, vec![frame_press(git, false)]);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    let split = frame_center(&app, &ctx, "pane-split-horizontal:right");
    combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(split)]);
    combined_frame(&mut app, &ctx, vec![frame_press(split, true)]);
    combined_frame(&mut app, &ctx, vec![frame_press(split, false)]);
    assert_eq!(
        app.add_tab
            .as_ref()
            .map(|(_, direction)| direction.as_deref()),
        Some(Some("down"))
    );
}
