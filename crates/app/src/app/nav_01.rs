use eframe::egui::{self};
use egui_dock::NodeIndex;
use std::sync::mpsc::{self};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_and_menu_shortcuts_run_their_actions() {
        let (mut app, ctx, _dir) = fixture();
        assert!(app.preferences.left_visible);
        assert!(app.preferences.visible);
        app.run_shortcut(&ctx, "toggle_left_sidebar");
        app.run_shortcut(&ctx, "toggle_right_sidebar");
        assert!(!app.preferences.left_visible);
        assert!(!app.preferences.visible);
        app.state.sessions = vec![session_fixture("live", SessionKind::Shell)];
        app.active_session = Some("live".into());
        app.run_shortcut(&ctx, "search_scrollback");
        assert!(app.search_open);
        assert_eq!(app.search_session.as_deref(), Some("live"));
        app.run_shortcut(&ctx, "rename_terminal");
        assert_eq!(
            app.rename_session.as_ref().map(|(id, _)| id.as_str()),
            Some("live")
        );
        app.rename_session = None;
        app.run_shortcut(&ctx, "close_session");
        assert_eq!(app.close_session.as_deref(), Some("live"));
        assert!(app.active_editor_id().is_none());
        app.run_shortcut(&ctx, "editor_save");
        assert_eq!(app.close_session.as_deref(), Some("live"));
    }

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
        app.run_shortcut(&ctx, "toggle_left_sidebar");
        let right_only = draw(&mut app);
        assert!(right_only > both + 100.0);
        app.run_shortcut(&ctx, "toggle_right_sidebar");
        let neither = draw(&mut app);
        assert!(neither > right_only + 100.0);
        app.run_shortcut(&ctx, "toggle_left_sidebar");
        app.run_shortcut(&ctx, "toggle_right_sidebar");
        assert!((draw(&mut app) - both).abs() < 1.0);
        assert!(app.preferences.ide_mode);
    }

    #[test]
    fn toggle_ide_mode_keeps_sidebar_visibility_and_widths() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.left_visible = false;
        app.preferences.visible = false;
        app.preferences.width = 300.0;
        app.preferences.tool = SidebarTool::Git;
        assert!(!app.preferences.ide_mode);
        app.run_shortcut(&ctx, "toggle_ide_mode");
        assert!(app.preferences.ide_mode);
        assert!(!app.preferences.left_visible);
        assert!(!app.preferences.visible);
        assert_eq!(app.preferences.width, 300.0);
        assert_eq!(app.preferences.tool, SidebarTool::Git);
        assert!(!app.preferences.ide_terminal_collapsed);
        app.run_shortcut(&ctx, "toggle_ide_mode");
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
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
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
        app.run_shortcut(&ctx, "split_right");
        let Job::Control(_, After::StripAt(_, split)) = requests.try_recv().unwrap() else {
            panic!("Strip-focused splits stay in the strip");
        };
        assert_eq!(split.as_deref(), Some("right"));
        app.active_session = Some("main".into());
        app.run_shortcut(&ctx, "split_right");
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
        main.main_surface_mut().split_right(
            NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("dock".into())],
        );
        app.layouts.insert("a".into(), Workspace::from_layout(main));
        app.preferences.ide_mode = true;
        app.active_session = Some("strip".into());
        app.run_shortcut(&ctx, "toggle_ide_mode");
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
        app.run_shortcut(&ctx, "toggle_ide_mode");
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
        app.run_shortcut(&ctx, "move_to_main");
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
        app.run_shortcut(&ctx, "move_to_main");
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
}
