use super::*;

#[test]
fn ide_sidebar_toggles_reclaim_layout_space_and_restore_it() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.ide_mode = true;
    let draw = |app: &mut App| {
        let mut width = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.workspace_sidebars(ui);
                width = ui.available_width();
            },
        );
        output.textures_delta.clear();
        width
    };
    draw(&mut app);
    let both = draw(&mut app);
    app.run_shortcut(&ctx, Action::ToggleLeftSidebar);
    let right_only = draw(&mut app);
    assert!(right_only > both + 100.0);
    app.run_shortcut(&ctx, Action::ToggleRightSidebar);
    let neither = draw(&mut app);
    assert!(neither > right_only + 100.0);
    app.run_shortcut(&ctx, Action::ToggleLeftSidebar);
    app.run_shortcut(&ctx, Action::ToggleRightSidebar);
    assert!((draw(&mut app) - both).abs() < 1.0);
    assert!(app.preferences.ide_mode);
}

#[test]
#[cfg(feature = "test-support")]
fn ide_stripes_switch_views_and_toggle_tools() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.ide_mode = true;
    let draw = |app: &mut App, events: &[egui::Event]| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 600.0),
                )),
                events: events.to_vec(),
                ..Default::default()
            },
            |ui| app.workspace_sidebars(ui),
        );
        output.textures_delta.clear();
    };
    draw(&mut app, &[]);
    draw(&mut app, &[]);
    let projects = header_target(&ctx)("stripe-projects").expect("projects stripe");
    let agents = header_target(&ctx)("stripe-agents").expect("agents stripe");
    let git = header_target(&ctx)("stripe-tool-Git").expect("git stripe");
    let search = header_target(&ctx)("stripe-search").expect("search stripe");
    assert!(projects.left() < 44.0 && agents.left() < 44.0);
    assert!(git.right() > 1156.0 && search.left() < 44.0);
    let click = |app: &mut App, rect: egui::Rect| {
        let pos = rect.center();
        draw(app, &[egui::Event::PointerMoved(pos)]);
        draw(
            app,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        draw(
            app,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
    };
    click(&mut app, agents);
    assert!(app.preferences.left_visible && app.preferences.left_agents);
    click(&mut app, projects);
    assert!(app.preferences.left_visible && !app.preferences.left_agents);
    click(&mut app, git);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    assert!(app.preferences.visible);
    click(&mut app, git);
    assert!(!app.preferences.visible);
    click(&mut app, search);
    assert!(app.palette.open);
    // Classic mode keeps the same stripes; only the terminal strip
    // is IDE-only. Regression: Explorer/Git had no access point here.
    app.palette.open = false;
    app.preferences.ide_mode = false;
    draw(&mut app, &[]);
    draw(&mut app, &[]);
    let git_classic = header_target(&ctx)("stripe-tool-Git").expect("git stripe (classic)");
    let explorer_classic =
        header_target(&ctx)("stripe-tool-Explorer").expect("explorer stripe (classic)");
    click(&mut app, git_classic);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    click(&mut app, explorer_classic);
    assert_eq!(app.preferences.tool, SidebarTool::Explorer);
    assert!(app.preferences.visible);
    // Every tool is on both stripes; the side clicked hosts the view.
    let left_git = header_target(&ctx)("stripe-left-Git").expect("left git stripe");
    let right_projects =
        header_target(&ctx)("stripe-right-Projects").expect("right projects stripe");
    let right_search = header_target(&ctx)("stripe-right-Search").expect("right search");
    click(&mut app, left_git);
    assert_eq!(app.preferences.left_tool, Some(SidebarTool::Git));
    assert!(app.preferences.left_visible);
    assert_eq!(app.preferences.tool, SidebarTool::Explorer);
    click(&mut app, left_git);
    assert!(!app.preferences.left_visible);
    click(&mut app, right_projects);
    assert!(app.preferences.right_projects);
    assert!(app.preferences.visible);
    click(&mut app, explorer_classic);
    assert!(!app.preferences.right_projects);
    assert_eq!(app.preferences.tool, SidebarTool::Explorer);
    click(&mut app, right_search);
    assert!(app.palette.open);
    // Same view on both sides at once (Projects left + right) must not
    // clash widget IDs: the right copy's filter takes focus and text.
    app.palette.open = false;
    app.preferences.left_tool = None;
    app.preferences.left_agents = false;
    app.preferences.left_visible = true;
    app.preferences.right_projects = true;
    app.preferences.visible = true;
    draw(&mut app, &[]);
    draw(&mut app, &[]);
    let right_filter = header_target(&ctx)("project-filter").expect("right filter");
    click(&mut app, right_filter);
    draw(&mut app, &[egui::Event::Text("b".into())]);
    assert_eq!(app.preferences.project_filter, "b");
    assert!(app.text_input_focused(&ctx));
}

#[test]
fn first_state_load_in_ide_mode_moves_shells_to_the_strip() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = false;
    assert!(app.preferences.ide_mode, "fresh preferences are IDE mode");
    let main = egui_dock::DockState::new(vec![Tab::Terminal("shell".into())]);
    let layout = serde_json::to_value(Workspace::from_layout(main)).unwrap();
    app.apply_state(State {
        projects: vec![Project {
            id: "a".into(),
            name: "a".into(),
            path: PathBuf::from("/a"),
            layout,
        }],
        sessions: vec![session_fixture("shell", SessionKind::Shell)],
        selected_project: Some("a".into()),
        ..Default::default()
    });
    assert!(app.is_strip_session("a", "shell"));
    assert!(!app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    // A later state load must not drag shells back out of the main dock.
    app.move_project_strip_to_main("a");
    assert!(!app.is_strip_session("a", "shell"));
    let mut again = app.state.clone();
    again.revision = again.revision.wrapping_add(1);
    app.apply_state(again);
    assert!(!app.is_strip_session("a", "shell"));
}

#[test]
fn toggle_ide_mode_keeps_sidebar_visibility_and_widths() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.left_visible = false;
    app.preferences.visible = false;
    app.preferences.width = 300.0;
    app.preferences.tool = SidebarTool::Git;
    assert!(!app.preferences.ide_mode);
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    assert!(app.preferences.ide_mode);
    assert!(!app.preferences.left_visible);
    assert!(!app.preferences.visible);
    assert_eq!(app.preferences.width, 300.0);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    assert!(!app.preferences.ide_terminal_collapsed);
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    assert!(!app.preferences.ide_mode);
    assert!(!app.preferences.left_visible);
    assert!(!app.preferences.visible);
}

#[test]
fn strip_creation_lands_in_the_strip_not_the_dock() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    let (updates, rx) = mpsc::channel();
    app.updates = rx;
    updates
        .send(Update::StripCreated(
            session_fixture("new-strip", SessionKind::Shell),
            None,
            Vec::new(),
        ))
        .unwrap();
    pump_until(&mut app, &ctx, |app| {
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .is_some_and(|dock| dock.find_tab(&Tab::Terminal("new-strip".into())).is_some())
    });
    assert!(
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .is_some_and(|dock| dock.find_tab(&Tab::Terminal("new-strip".into())).is_some())
    );
    assert_eq!(app.active_session.as_deref(), Some("new-strip"));
    assert!(app.state.sessions.iter().any(|s| s.id == "new-strip"));
    assert!(
        !app.layouts
            .get("a")
            .is_some_and(|dock| dock.contains(&Tab::Terminal("new-strip".into())))
    );
}

#[test]
fn strip_split_creation_splits_the_strip_leaf() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("one", SessionKind::Shell),
        session_fixture("two", SessionKind::Shell),
    ];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("one".into())]),
    );
    let (updates, rx) = mpsc::channel();
    app.updates = rx;
    updates
        .send(Update::StripCreated(
            session_fixture("two", SessionKind::Shell),
            Some("right".into()),
            vec![Tab::Terminal("one".into())],
        ))
        .unwrap();
    pump_until(&mut app, &ctx, |app| {
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .is_some_and(|dock| dock.find_tab(&Tab::Terminal("two".into())).is_some())
    });
    let dock = app.preferences.ide_strip_docks.0.get("a").unwrap();
    assert_eq!(dock.iter_leaves().count(), 2);
    assert!(dock.find_tab(&Tab::Terminal("two".into())).is_some());
    assert!(
        !app.layouts
            .get("a")
            .is_some_and(|dock| dock.contains(&Tab::Terminal("two".into())))
    );
}

#[test]
fn strip_plus_adds_the_tab_to_the_clicked_pane() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("left", SessionKind::Shell),
        session_fixture("right", SessionKind::Shell),
    ];
    let mut dock = egui_dock::DockState::new(vec![Tab::Terminal("left".into())]);
    let [left, right] = dock.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    dock.main_surface_mut().set_focused_node(left);
    let right_path = egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: right,
    };
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.add_strip_tab = Some((right_path, None));
    app.apply_add_strip_tab("a", &mut dock);
    app.preferences.ide_strip_docks.0.insert("a".into(), dock);
    let Job::Control(_, After::StripAt(anchors, None)) = requests.try_recv().unwrap() else {
        panic!("Plain strip + anchors on the clicked pane");
    };
    let (updates, rx) = mpsc::channel();
    app.updates = rx;
    updates
        .send(Update::StripCreated(
            session_fixture("new", SessionKind::Shell),
            None,
            anchors,
        ))
        .unwrap();
    pump_until(&mut app, &ctx, |app| {
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .is_some_and(|dock| dock.find_tab(&Tab::Terminal("new".into())).is_some())
    });
    let dock = app.preferences.ide_strip_docks.0.get("a").unwrap();
    let path = dock.find_tab(&Tab::Terminal("new".into())).unwrap();
    assert_eq!(path.node_path().node, right);
}

#[test]
fn splits_follow_the_focused_dock() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("strip", SessionKind::Shell),
        session_fixture("main", SessionKind::Shell),
    ];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.active_session = Some("strip".into());
    app.run_shortcut(&ctx, Action::SplitRight);
    let Job::Control(_, After::StripAt(_, split)) = requests.try_recv().unwrap() else {
        panic!("Strip-focused splits stay in the strip");
    };
    assert_eq!(split.as_deref(), Some("right"));
    app.active_session = Some("main".into());
    app.run_shortcut(&ctx, Action::SplitRight);
    let Job::Control(_, After::CreateAt(_, split)) = requests.try_recv().unwrap() else {
        panic!("Main-focused splits stay in the main dock");
    };
    assert_eq!(split.as_deref(), Some("right"));
}

#[test]
fn regular_mode_opens_a_strip_shell_in_the_main_dock() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![session_fixture("strip", SessionKind::Shell)];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    assert!(!app.preferences.ide_mode);
    app.go_session("strip");
    assert!(!app.preferences.ide_mode);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("strip".into())));
    assert!(!app.is_strip_session("a", "strip"));
}

#[test]
fn ide_mode_session_jump_still_focuses_the_strip() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.preferences.ide_terminal_collapsed = true;
    app.state.sessions = vec![session_fixture("strip", SessionKind::Shell)];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    app.go_session("strip");
    assert!(app.preferences.ide_mode);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    assert!(!app.layouts["a"].contains(&Tab::Terminal("strip".into())));
}

#[test]
fn sync_follows_dock_focus_moves_but_keeps_strip_clicks() {
    let (mut app, _ctx, _dir) = fixture();
    let mut dock = Workspace::from_layout(egui_dock::DockState::new(vec![
        Tab::Terminal("t".into()),
        Tab::Terminal("u".into()),
    ]));
    let first = dock.find_tab(&Tab::Terminal("t".into())).unwrap();
    dock.set_focused_node_and_surface(first.node_path());
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("t"));
    // A strip click selects a session with no dock tab; unchanged dock
    // focus must keep it instead of clobbering it back to the dock.
    app.active_session = Some("strip".into());
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    // Moving dock focus to another tab takes over again.
    let path = dock.find_tab(&Tab::Terminal("u".into())).unwrap();
    let _ = dock.set_active_tab(path);
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("u"));
    // A hidden strip records focus and must not take the visible session.
    app.selected = Some("a".into());
    let mut strip =
        egui_dock::DockState::new(vec![Tab::Terminal("s".into()), Tab::Terminal("s2".into())]);
    let spath = strip.find_tab(&Tab::Terminal("s".into())).unwrap();
    strip.set_focused_node_and_surface(spath.node_path());
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("u"));
    // Revealing the strip does not replay the focus recorded while hidden.
    app.preferences.ide_mode = true;
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("u"));
    // A later move while the strip is on screen takes over.
    let s2 = app
        .preferences
        .ide_strip_docks
        .0
        .get("a")
        .unwrap()
        .find_tab(&Tab::Terminal("s2".into()))
        .unwrap();
    let _ = app
        .preferences
        .ide_strip_docks
        .0
        .get_mut("a")
        .unwrap()
        .set_active_tab(s2);
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("s2"));
}

#[test]
fn hidden_strip_keeps_the_visible_terminal_across_projects() {
    let (mut app, _ctx, _dir) = fixture();
    let mut dock_a = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "main-a".into(),
    )]));
    let main_a = dock_a.find_tab(&Tab::Terminal("main-a".into())).unwrap();
    dock_a.set_focused_node_and_surface(main_a.node_path());
    app.selected = Some("a".into());
    let mut strip_a = egui_dock::DockState::new(vec![Tab::Terminal("strip-a".into())]);
    let path = strip_a.find_tab(&Tab::Terminal("strip-a".into())).unwrap();
    strip_a.set_focused_node_and_surface(path.node_path());
    app.preferences
        .ide_strip_docks
        .0
        .insert("a".into(), strip_a);
    app.sync_active_session(&mut dock_a);
    assert_eq!(app.active_session.as_deref(), Some("main-a"));

    app.selected = Some("b".into());
    let mut dock_b = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "main-b".into(),
    )]));
    let main_b = dock_b.find_tab(&Tab::Terminal("main-b".into())).unwrap();
    dock_b.set_focused_node_and_surface(main_b.node_path());
    let mut strip_b = egui_dock::DockState::new(vec![Tab::Terminal("strip-b".into())]);
    let path = strip_b.find_tab(&Tab::Terminal("strip-b".into())).unwrap();
    strip_b.set_focused_node_and_surface(path.node_path());
    app.preferences
        .ide_strip_docks
        .0
        .insert("b".into(), strip_b);
    app.preferences.ide_terminal_collapsed = true;
    app.preferences.ide_mode = true;
    app.sync_active_session(&mut dock_b);
    assert_eq!(app.active_session.as_deref(), Some("main-b"));
}

#[test]
fn visible_strip_keeps_the_main_terminal_across_projects() {
    let (mut app, _ctx, _dir) = fixture();
    app.preferences.ide_mode = true;
    let mut dock_a = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "main-a".into(),
    )]));
    let main_a = dock_a.find_tab(&Tab::Terminal("main-a".into())).unwrap();
    dock_a.set_focused_node_and_surface(main_a.node_path());
    app.selected = Some("a".into());
    let mut strip_a = egui_dock::DockState::new(vec![Tab::Terminal("strip-a".into())]);
    let path = strip_a.find_tab(&Tab::Terminal("strip-a".into())).unwrap();
    strip_a.set_focused_node_and_surface(path.node_path());
    app.preferences
        .ide_strip_docks
        .0
        .insert("a".into(), strip_a);
    app.sync_active_session(&mut dock_a);
    assert_eq!(app.active_session.as_deref(), Some("main-a"));

    app.selected = Some("b".into());
    let mut dock_b = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "main-b".into(),
    )]));
    let main_b = dock_b.find_tab(&Tab::Terminal("main-b".into())).unwrap();
    dock_b.set_focused_node_and_surface(main_b.node_path());
    let mut strip_b = egui_dock::DockState::new(vec![
        Tab::Terminal("strip-b".into()),
        Tab::Terminal("strip-b2".into()),
    ]);
    let path = strip_b.find_tab(&Tab::Terminal("strip-b".into())).unwrap();
    strip_b.set_focused_node_and_surface(path.node_path());
    app.preferences
        .ide_strip_docks
        .0
        .insert("b".into(), strip_b);
    app.sync_active_session(&mut dock_b);
    assert_eq!(app.active_session.as_deref(), Some("main-b"));

    let next = app
        .preferences
        .ide_strip_docks
        .0
        .get("b")
        .unwrap()
        .find_tab(&Tab::Terminal("strip-b2".into()))
        .unwrap();
    let _ = app
        .preferences
        .ide_strip_docks
        .0
        .get_mut("b")
        .unwrap()
        .set_active_tab(next);
    app.sync_active_session(&mut dock_b);
    assert_eq!(app.active_session.as_deref(), Some("strip-b2"));
}

#[test]
fn closing_a_strip_terminal_recovers_past_a_remembered_image() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("s1", SessionKind::Shell),
        session_fixture("s2", SessionKind::Shell),
    ];
    let image = Tab::Image {
        path: "/tmp/a.png".into(),
    };
    let mut dock = Workspace::from_layout(egui_dock::DockState::new(vec![image.clone()]));
    let path = dock.find_tab(&image).unwrap();
    dock.set_focused_node_and_surface(path.node_path());
    let mut strip =
        egui_dock::DockState::new(vec![Tab::Terminal("s1".into()), Tab::Terminal("s2".into())]);
    let focused = strip.find_tab(&Tab::Terminal("s2".into())).unwrap();
    strip.set_focused_node_and_surface(focused.node_path());
    let _ = strip.set_active_tab(focused);
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session, None);

    // The strip terminal is the selection. The image stays the main pane.
    app.active_session = Some("s1".into());
    app.remove_tab("s1");
    app.sync_active_session(&mut dock);
    app.restore_cleared_focus(&dock);
    assert_eq!(app.active_session.as_deref(), Some("s2"));
}

#[test]
fn selecting_a_non_terminal_stays_unselected() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![session_fixture("strip", SessionKind::Shell)];
    let mut strip = egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]);
    let path = strip.find_tab(&Tab::Terminal("strip".into())).unwrap();
    strip.set_focused_node_and_surface(path.node_path());
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    // The strip is already focused, so selecting a pane is not a strip move.
    let mut primer = Workspace::empty();
    app.sync_active_session(&mut primer);
    let panes = [
        Tab::Image {
            path: "/tmp/a.png".into(),
        },
        Tab::Browser {
            id: "b".into(),
            target: BrowserTarget::Url("https://example.com".into()),
        },
        Tab::Diff {
            remote: None,
            cwd: "/tmp".into(),
            path: "/tmp/a.rs".into(),
            staged: false,
        },
        Tab::Player,
        Tab::NativeEditor {
            path: "/tmp/a.rs".into(),
        },
    ];
    for pane in panes {
        let mut dock = Workspace::from_layout(egui_dock::DockState::new(vec![pane.clone()]));
        let path = dock.find_tab(&pane).unwrap();
        dock.set_focused_node_and_surface(path.node_path());
        app.active_session = Some("strip".into());
        app.sync_active_session(&mut dock);
        assert_eq!(app.active_session, None, "{pane:?}");
        app.restore_cleared_focus(&dock);
        assert_eq!(app.active_session, None, "{pane:?}");
        app.restore_cleared_focus(&dock);
        assert_eq!(app.active_session, None, "{pane:?}");
    }
}

#[test]
fn sync_survives_an_emptied_strip_dock() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    // Exiting the last strip terminal empties the tree while focus goes stale.
    let mut strip = egui_dock::DockState::new(vec![Tab::Terminal("s".into())]);
    let path = strip.find_tab(&Tab::Terminal("s".into())).unwrap();
    strip.set_focused_node_and_surface(path.node_path());
    strip.remove_tab(path);
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.active_session = Some("elsewhere".into());
    let mut dock = Workspace::empty();
    app.sync_active_session(&mut dock);
    assert_eq!(app.active_session.as_deref(), Some("elsewhere"));
}

#[test]
fn cleared_focus_returns_to_the_strip_then_the_dock() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![
        session_fixture("dock", SessionKind::Shell),
        session_fixture("strip", SessionKind::Shell),
    ];
    app.preferences.ide_mode = true;
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    let dock = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "dock".into(),
    )]));
    app.active_session = None;
    app.restore_cleared_focus(&dock);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    // No live strip tabs: fall back to the dock's focused terminal.
    app.preferences
        .ide_strip_docks
        .0
        .insert("a".into(), egui_dock::DockState::new(vec![]));
    app.active_session = None;
    app.restore_cleared_focus(&dock);
    assert_eq!(app.active_session.as_deref(), Some("dock"));
}

#[test]
fn leaving_ide_mode_puts_the_focused_strip_shell_on_the_main_screen() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![
        session_fixture("dock", SessionKind::Shell),
        session_fixture("strip", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    let mut main = egui_dock::DockState::new(vec![Tab::Terminal("edit".into())]);
    main.main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![Tab::Terminal("dock".into())]);
    app.layouts.insert("a".into(), Workspace::from_layout(main));
    app.preferences.ide_mode = true;
    app.active_session = Some("strip".into());
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    assert!(!app.preferences.ide_mode);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("strip".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("edit".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("dock".into())));
    assert!(!app.is_strip_session("a", "strip"));
    // Sidebar jump stays in regular mode and shows the same shell.
    app.go_session("strip");
    assert!(!app.preferences.ide_mode);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
}

#[test]
fn entering_ide_mode_puts_main_shells_in_the_strip_and_keeps_editors() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![
        session_fixture("shell", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    let mut main = egui_dock::DockState::new(vec![Tab::Terminal("edit".into())]);
    main.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("shell".into())],
    );
    app.layouts.insert("a".into(), Workspace::from_layout(main));
    app.active_session = Some("shell".into());
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    assert!(app.preferences.ide_mode);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert_eq!(app.active_session.as_deref(), Some("shell"));
    assert!(app.is_strip_session("a", "shell"));
    assert!(!app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("edit".into())));
}

#[test]
fn move_to_main_keeps_ide_mode_and_the_strip_sibling() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("left", SessionKind::Shell),
        session_fixture("right", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    let mut strip = egui_dock::DockState::new(vec![Tab::Terminal("left".into())]);
    strip.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
            "edit".into(),
        )])),
    );
    app.active_session = Some("right".into());
    app.run_shortcut(&ctx, Action::MoveToMain);
    assert!(app.preferences.ide_mode);
    assert_eq!(app.active_session.as_deref(), Some("right"));
    assert!(!app.is_strip_session("a", "right"));
    assert!(app.is_strip_session("a", "left"));
    assert_eq!(
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .unwrap()
            .iter_all_tabs()
            .count(),
        1
    );
    let workspace = &app.layouts["a"];
    assert!(workspace.tabs.iter().any(|tab| {
        tab.layout.find_tab(&Tab::Terminal("edit".into())).is_some()
            && tab
                .layout
                .find_tab(&Tab::Terminal("right".into()))
                .is_none()
    }));
    assert!(workspace.tabs.iter().any(|tab| {
        tab.layout
            .find_tab(&Tab::Terminal("right".into()))
            .is_some()
            && tab.layout.find_tab(&Tab::Terminal("edit".into())).is_none()
    }));
    assert_eq!(
        workspace.active_pane(),
        Some(&Tab::Terminal("right".into()))
    );
    // The shell is already in the main pane. The chord does not copy it.
    app.run_shortcut(&ctx, Action::MoveToMain);
    assert_eq!(
        app.layouts["a"]
            .tabs
            .iter()
            .filter(|tab| tab
                .layout
                .find_tab(&Tab::Terminal("right".into()))
                .is_some())
            .count(),
        1
    );
    assert!(app.is_strip_session("a", "left"));
    assert!(!app.move_strip_session_to_main("edit"));
    assert!(!app.move_strip_session_to_main("missing"));
}

#[test]
fn queued_strip_move_waits_until_the_dock_is_checked_in() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("left", SessionKind::Shell),
        session_fixture("right", SessionKind::Shell),
    ];
    let mut strip = egui_dock::DockState::new(vec![Tab::Terminal("left".into())]);
    strip.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.active_session = Some("right".into());
    app.queue_strip_move("left");
    let checked_out = app.preferences.ide_strip_docks.0.remove("a").unwrap();
    app.drain_strip_move();
    assert_eq!(app.move_strip_to_main.as_deref(), Some("left"));
    assert!(
        checked_out
            .find_tab(&Tab::Terminal("left".into()))
            .is_some()
    );
    app.preferences
        .ide_strip_docks
        .0
        .insert("a".into(), checked_out);
    app.drain_strip_move();
    assert!(app.move_strip_to_main.is_none());
    assert!(!app.is_strip_session("a", "left"));
    assert!(app.is_strip_session("a", "right"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("left".into())));
    assert_eq!(app.active_session.as_deref(), Some("left"));
    assert!(app.preferences.ide_mode);
}

#[test]
fn move_to_strip_keeps_ide_mode_and_the_main_editor() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.preferences.ide_terminal_collapsed = true;
    app.state.sessions = vec![
        session_fixture("dock", SessionKind::Shell),
        session_fixture("stay", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    let mut main = egui_dock::DockState::new(vec![Tab::Terminal("edit".into())]);
    main.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("dock".into()), Tab::Terminal("stay".into())],
    );
    app.layouts.insert("a".into(), Workspace::from_layout(main));
    app.preferences
        .ide_strip_docks
        .0
        .insert("a".into(), egui_dock::DockState::new(vec![]));
    app.active_session = Some("dock".into());
    app.run_shortcut(&ctx, Action::MoveToStrip);
    assert!(app.preferences.ide_mode);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert_eq!(app.active_session.as_deref(), Some("dock"));
    assert!(app.is_strip_session("a", "dock"));
    assert!(!app.is_strip_session("a", "stay"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("edit".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("stay".into())));
    assert!(!app.layouts["a"].contains(&Tab::Terminal("dock".into())));
    // Already in the strip. The chord does not copy it, and IDE mode
    // off has no lower pane to move into.
    app.run_shortcut(&ctx, Action::MoveToStrip);
    assert_eq!(
        app.preferences
            .ide_strip_docks
            .0
            .get("a")
            .unwrap()
            .iter_all_tabs()
            .filter(|(_, tab)| matches!(tab, Tab::Terminal(sid) if sid == "dock"))
            .count(),
        1
    );
    app.preferences.ide_mode = false;
    app.active_session = Some("stay".into());
    assert!(!app.move_main_session_to_strip("stay"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("stay".into())));
}

#[test]
fn strip_drag_lands_on_the_main_leaf() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.preferences.ide_mode = true;
    app.state.sessions = vec![
        session_fixture("left", SessionKind::Shell),
        session_fixture("edit", SessionKind::Editor),
    ];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("left".into())]),
    );
    let mut dock = Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
        "edit".into(),
    )]));
    let path = dock
        .find_tab(&Tab::Terminal("edit".into()))
        .unwrap()
        .node_path();
    let group = dock.active.clone();
    assert!(app.land_strip_shell_on_leaf(&mut dock, "left", &group, path, PaneDropZone::Center));
    assert!(app.preferences.ide_mode);
    assert!(!app.is_strip_session("a", "left"));
    assert!(dock.contains(&Tab::Terminal("left".into())));
    assert!(dock.contains(&Tab::Terminal("edit".into())));
    assert!(app.pending_layout_save);
}

#[test]
fn strip_split_survives_a_mode_round_trip() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![
        session_fixture("left", SessionKind::Shell),
        session_fixture("right", SessionKind::Shell),
    ];
    let mut strip = egui_dock::DockState::new(vec![Tab::Terminal("left".into())]);
    strip.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.preferences.ide_strip_docks.0.insert("a".into(), strip);
    app.preferences.ide_mode = true;
    app.active_session = Some("right".into());
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    assert_eq!(app.layouts["a"].iter_leaves().count(), 2);
    assert_eq!(app.active_session.as_deref(), Some("right"));
    app.run_shortcut(&ctx, Action::ToggleIdeMode);
    let strip = app.preferences.ide_strip_docks.0.get("a").unwrap();
    assert_eq!(strip.iter_leaves().count(), 2);
    assert!(strip.find_tab(&Tab::Terminal("left".into())).is_some());
    assert!(strip.find_tab(&Tab::Terminal("right".into())).is_some());
    assert!(!app.layouts["a"].contains(&Tab::Terminal("left".into())));
    assert_eq!(app.active_session.as_deref(), Some("right"));
}

#[test]
fn remove_tab_drops_strip_membership() {
    let (mut app, _ctx, _dir) = fixture();
    for (project, tabs) in [
        (
            "a",
            vec![Tab::Terminal("x".into()), Tab::Terminal("y".into())],
        ),
        ("b", vec![Tab::Terminal("x".into())]),
    ] {
        app.preferences
            .ide_strip_docks
            .0
            .insert(project.into(), egui_dock::DockState::new(tabs));
    }
    app.remove_tab("x");
    let has = |project: &str, sid: &str| {
        app.preferences
            .ide_strip_docks
            .0
            .get(project)
            .is_some_and(|dock| dock.find_tab(&Tab::Terminal(sid.into())).is_some())
    };
    assert!(!has("a", "x"));
    assert!(has("a", "y"));
    assert!(!has("b", "x"));
}

#[test]
#[cfg(feature = "test-support")]
fn ide_strip_trailing_controls_share_one_row() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state
        .sessions
        .push(session_fixture("s1", SessionKind::Shell));
    app.active_session = Some("s1".into());
    app.metadata = Some(metadata::Metadata {
        cwd: "/a".into(),
        branch: Some("master".into()),
        ..Default::default()
    });
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("s1".into())]),
    );
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| app.ide_terminal_strip(ui),
    );
    output.textures_delta.clear();
    let mut centers = Vec::new();
    for name in [
        "pane-git:s1",
        "pane-split-vertical:s1",
        "pane-split-horizontal:s1",
        "pane-dropdown",
    ] {
        let rect = app
            .fixture_rect(&ctx, name)
            .unwrap_or_else(|| panic!("missing {name}"));
        centers.push((name, rect[1] + rect[3] / 2.0, rect[2], rect[3]));
    }
    crate::console::Console::err(format_args!("{centers:#?}"));
    let (_, base, _, _) = centers[0];
    for (name, center, _, _) in &centers {
        assert!(
            (center - base).abs() < 1.0,
            "{name} center {center} should equal {base}"
        );
    }
}

#[test]
#[cfg(feature = "test-support")]
fn ide_terminal_strip_renders_without_sessions() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| app.ide_terminal_strip(ui),
    );
    output.textures_delta.clear();
    assert!(agent_target(&ctx, "ide-terminal-strip").is_some());
}

#[test]
#[cfg(feature = "test-support")]
fn ide_sidebar_height_moves_the_terminal_strip() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.ide_mode = true;
    app.preferences.ide_terminal_collapsed = false;
    app.preferences.left_visible = true;
    app.preferences.visible = true;
    app.selected = Some("a".into());
    fn paint(app: &mut App, ctx: &egui::Context) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| app.place_ide_columns(ui),
        );
        output.textures_delta.clear();
    }
    app.preferences.ide_sidebars_full_height = true;
    paint(&mut app, &ctx);
    let left = agent_target(&ctx, "projects-sidebar").expect("left sidebar");
    let right = agent_target(&ctx, "context-sidebar").expect("right sidebar");
    let strip = agent_target(&ctx, "ide-terminal-panel").expect("terminal strip");
    assert!(
        left.bottom() > strip.center().y && right.bottom() > strip.center().y,
        "full-height sidebars should run through the strip: left {left:?} right {right:?} strip {strip:?}"
    );
    assert!(
        strip.left() + 1.0 >= left.right() && strip.right() <= right.left() + 1.0,
        "strip should stay between the sidebars: left {left:?} right {right:?} strip {strip:?}"
    );

    app.preferences.ide_sidebars_full_height = false;
    paint(&mut app, &ctx);
    let left = agent_target(&ctx, "projects-sidebar").expect("left sidebar");
    let right = agent_target(&ctx, "context-sidebar").expect("right sidebar");
    let strip = agent_target(&ctx, "ide-terminal-panel").expect("terminal strip");
    assert!(
        left.bottom() <= strip.top() + 4.0 && right.bottom() <= strip.top() + 4.0,
        "sidebars should stop above a full-width strip: left {left:?} right {right:?} strip {strip:?}"
    );
    assert!(
        strip.left() <= left.left() + 1.0 && strip.right() + 1.0 >= right.right(),
        "strip should run under both sidebars: left {left:?} right {right:?} strip {strip:?}"
    );
}

#[test]
fn strip_tabs_reserve_leading_icons_for_every_terminal() {
    use egui_dock::TabViewer;
    let (mut app, _ctx, _dir) = fixture();
    app.state.sessions = vec![
        session_fixture("agent", SessionKind::Shell),
        session_fixture("plain", SessionKind::Shell),
    ];
    app.state.agents = vec![Agent {
        invocation_id: "agent".into(),
        session_id: "agent".into(),
        kind: "codex".into(),
        provider_session_id: None,
        state: AgentState::Running,
        sequence: None,
        updated: 1,
        resume: None,
        process: None,
    }];
    let viewer = Viewer {
        app: &mut app,
        strip: true,
        project: None,
        tab: None,
        window: None,
        render_path: None,
        render_tab: None,
    };
    let agent_width = viewer.tab_leading_width(&Tab::Terminal("agent".into()));
    let plain_width = viewer.tab_leading_width(&Tab::Terminal("plain".into()));
    assert!(
        agent_width > plain_width,
        "an agent tab carries brand plus status, a plain shell only its kind glyph"
    );
    assert!(
        plain_width > 0.0,
        "even a plain shell reserves a kind icon on strip tabs"
    );
    assert_eq!(
        viewer.tab_leading_width(&Tab::Terminal("missing".into())),
        0.0,
        "unknown sessions reserve no icon slot"
    );
    assert_eq!(
        viewer.tab_leading_width(&Tab::Player),
        0.0,
        "non-terminal tabs reserve no icon slot"
    );
}

#[test]
fn strip_tabs_show_a_detected_agent_before_any_hook() {
    use egui_dock::TabViewer;
    let (mut app, _ctx, _dir) = fixture();
    app.state
        .capabilities
        .push(AGENT_PRESENCE_CAPABILITY.into());
    app.state.sessions = vec![
        session_fixture("muse", SessionKind::Shell),
        session_fixture("plain", SessionKind::Shell),
    ];
    app.state.presence.push(agents::TerminalPresence {
        session_id: "muse".into(),
        generation: "fixture".into(),
        agents: vec![agents::DetectedAgent {
            kind: "muse".into(),
            process: agents::ProcessIdentity {
                pid: 9,
                start_time: 1,
            },
            foreground: true,
        }],
        outcome: agents::PresenceOutcome::Verified,
        observed_at: now(),
    });
    let viewer = Viewer {
        app: &mut app,
        strip: true,
        project: None,
        tab: None,
        window: None,
        render_path: None,
        render_tab: None,
    };
    let muse = viewer.tab_leading_width(&Tab::Terminal("muse".into()));
    let plain = viewer.tab_leading_width(&Tab::Terminal("plain".into()));
    assert_eq!(
        muse,
        plain + appearance::TERMINAL_LEADING_SLOT,
        "a process-detected agent reserves its brand before any hook"
    );
}
