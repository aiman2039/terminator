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
        app.run_shortcut(&ctx, "move_to_strip");
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
        app.run_shortcut(&ctx, "move_to_strip");
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
        assert!(app.land_strip_shell_on_leaf(
            &mut dock,
            "left",
            &group,
            path,
            PaneDropZone::Center
        ));
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
        app.run_shortcut(&ctx, "toggle_ide_mode");
        assert_eq!(app.layouts["a"].iter_leaves().count(), 2);
        assert_eq!(app.active_session.as_deref(), Some("right"));
        app.run_shortcut(&ctx, "toggle_ide_mode");
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
        eprintln!("{centers:#?}");
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
    fn explorer_toolbar_controls_share_one_row() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.tool = SidebarTool::Explorer;
        app.preferences.explorer_search_mode = crate::preferences::ExplorerSearchMode::Contents;
        let cwd = std::path::PathBuf::from("/a");
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 220.0),
                )),
                ..Default::default()
            },
            |ui| app.explorer_toolbar(ui, &cwd),
        );
        output.textures_delta.clear();
        for names in [
            [
                "explorer-new-file",
                "explorer-new-folder",
                "explorer-collapse",
                "explorer-refresh",
                "explorer-show-ignored",
                "explorer-more",
            ]
            .as_slice(),
            [
                "explorer-search",
                "explorer-match-case",
                "explorer-whole-word",
                "explorer-regex",
            ]
            .as_slice(),
        ] {
            let mut centers = Vec::new();
            for name in names {
                let rect = app
                    .fixture_rect(&ctx, name)
                    .unwrap_or_else(|| panic!("missing {name}"));
                centers.push((name, rect[1] + rect[3] / 2.0, rect[3]));
            }
            let (_, base, base_h) = centers[0];
            for (name, center, height) in &centers {
                assert!(
                    (center - base).abs() < 1.0,
                    "{name} center {center} should equal {base}"
                );
                assert!(
                    (height - base_h).abs() < 1.0,
                    "{name} height {height} should equal {base_h}"
                );
            }
        }
        let icons = app
            .fixture_rect(&ctx, "explorer-new-file")
            .expect("new file");
        let search = app.fixture_rect(&ctx, "explorer-search").expect("search");
        assert!(
            search[1] < icons[1] + icons[3] + 12.0,
            "search row stays under the toolbar, icons={} search={}",
            icons[1],
            search[1]
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn contents_search_queues_a_query() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.tool = SidebarTool::Explorer;
        app.preferences.explorer_search_mode = crate::preferences::ExplorerSearchMode::Contents;
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        let cwd = std::path::PathBuf::from("/a");
        let draw = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, 220.0),
                    )),
                    ..Default::default()
                },
                |ui| app.explorer_toolbar(ui, &cwd),
            );
            output.textures_delta.clear();
        };
        draw(&mut app);
        assert!(requests.try_recv().is_err(), "empty query queued a search");
        app.explorer_query = "needle".into();
        draw(&mut app);
        let job = requests
            .try_recv()
            .expect("non-empty query queues a search");
        assert!(matches!(job, Job::Search { .. }));
    }

    #[test]
    fn strip_input_stays_enabled_under_center_views() {
        let (mut app, _ctx, _dir) = fixture();
        assert!(app.strip_terminal_input_enabled("shell"));
        app.settings_open = true;
        assert!(app.strip_terminal_input_enabled("shell"));
        app.player_open = true;
        assert!(app.strip_terminal_input_enabled("shell"));
        app.palette_open = true;
        assert!(app.strip_terminal_input_enabled("shell"));
        app.palette_open = false;
        app.player_open = false;
        app.settings_open = false;
        // True modals still suspend the strip.
        app.add_project = true;
        assert!(!app.strip_terminal_input_enabled("shell"));
    }

    #[test]
    fn hover_popup_blocks_terminal_input_through_its_closing_frame() {
        let (mut app, ctx, _dir) = fixture();
        app.hover_popup = Some(HoverPopup {
            session: "shell".into(),
            key: "first-file".into(),
            target: services::Target::File("/first.rs".into(), None, None),
            rect: egui::Rect::ZERO,
        });
        assert!(!app.terminal_input_enabled("shell"));
        assert!(!app.strip_terminal_input_enabled("other-shell"));
        app.hover_popup_blocks_input = true;
        app.dismiss_hover_popup(&ctx);
        assert!(!app.terminal_input_enabled("shell"));
        assert!(!app.strip_terminal_input_enabled("other-shell"));
        app.hover_popup_blocks_input = false;
        assert!(app.terminal_input_enabled("shell"));
        assert!(app.strip_terminal_input_enabled("other-shell"));
    }

    #[test]
    fn dismissed_hover_stays_closed_until_the_pointer_leaves_its_file() {
        let (mut app, _ctx, _dir) = fixture();
        app.dismissed_hover = Some(("shell".into(), "first-file".into()));
        assert!(app.hover_target_dismissed("shell", "first-file"));
        assert!(!app.hover_target_dismissed("other-shell", "different-file"));
        assert!(app.hover_target_dismissed("shell", "first-file"));
        assert!(!app.hover_target_dismissed("shell", "outside"));
        assert!(!app.hover_target_dismissed("shell", "first-file"));
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
    fn ide_status_badge_renders_without_sidebars() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.ide_mode = true;
        app.preferences.left_visible = false;
        app.preferences.visible = false;
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 60.0),
                )),
                ..Default::default()
            },
            |ui| app.notification_status_badge(ui),
        );
        output.textures_delta.clear();
        assert!(agent_target(&ctx, "status-attention-bell").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn status_strip_shows_app_totals_and_opens_info() {
        let (mut app, ctx, _dir) = fixture();
        fn paint(app: &mut App, ctx: &egui::Context, events: &[egui::Event]) -> Vec<String> {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 30.0),
                    )),
                    events: events.to_vec(),
                    ..Default::default()
                },
                |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        app.app_resource_status(ui);
                    });
                },
            );
            output.textures_delta.clear();
            let mut text = Vec::new();
            for clipped in &output.shapes {
                walk_text(&clipped.shape, &mut text);
            }
            text
        }
        fn walk_text(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk_text(shape, out);
                    }
                }
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        // No sample yet: nothing painted.
        paint(&mut app, &ctx, &[]);
        assert!(agent_target(&ctx, "status-resources").is_none());

        // A sample that matched no app process: still nothing, never a
        // stuck zero.
        app.resources = Some(resource_sample::Sample {
            pid: None,
            started: 0,
            session: None,
            system: resource_sample::SystemStats {
                cpu: 0.0,
                memory_used: 0,
                memory_total: 0,
                pressure: None,
                load_one: 0.0,
                load_five: 0.0,
                load_fifteen: 0.0,
                cpus: 1,
            },
            app: resource_sample::AppStats::default(),
        });
        paint(&mut app, &ctx, &[]);
        assert!(agent_target(&ctx, "status-resources").is_none());

        app.resources.as_mut().unwrap().app = resource_sample::AppStats {
            gui: resource_sample::ComponentStats {
                cpu: 12.0,
                memory: 50 * 1024 * 1024,
                processes: 1,
            },
            daemon: resource_sample::ComponentStats {
                cpu: 3.0,
                memory: 100 * 1024 * 1024,
                processes: 1,
            },
            hooks: resource_sample::ComponentStats {
                cpu: 0.5,
                memory: 10 * 1024 * 1024,
                processes: 2,
            },
        };
        let text = paint(&mut app, &ctx, &[]);
        let rect = agent_target(&ctx, "status-resources").expect("resource readout");
        assert!(
            text.iter().any(|line| line.contains("16%")),
            "totals cpu {text:?}"
        );
        assert!(
            text.iter().any(|line| line.contains("160 MB")),
            "totals memory {text:?}"
        );
        let pos = rect.center();
        paint(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert_eq!(app.preferences.tool, SidebarTool::Info);
        assert!(app.preferences.visible);
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn status_resources_do_not_overlap_terminal_toggle() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.ide_mode = true;
        app.preferences.ide_terminal_collapsed = false;
        app.resources = Some(resource_sample::Sample {
            pid: None,
            started: 0,
            session: None,
            system: resource_sample::SystemStats {
                cpu: 0.0,
                memory_used: 0,
                memory_total: 0,
                pressure: None,
                load_one: 0.0,
                load_five: 0.0,
                load_fifteen: 0.0,
                cpus: 1,
            },
            app: resource_sample::AppStats {
                gui: resource_sample::ComponentStats {
                    cpu: 20.0,
                    memory: 300 * 1024 * 1024,
                    processes: 1,
                },
                daemon: resource_sample::ComponentStats {
                    cpu: 6.0,
                    memory: 50 * 1024 * 1024,
                    processes: 1,
                },
                hooks: resource_sample::ComponentStats::default(),
            },
        });
        for collapsed in [false, true] {
            app.preferences.ide_terminal_collapsed = collapsed;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 30.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label("status");
                        app.status_right_end(ui);
                    });
                },
            );
            output.textures_delta.clear();
            let resources = agent_target(&ctx, "status-resources").expect("resource readout");
            let toggle = agent_target(&ctx, "status-terminal-toggle").expect("terminal toggle");
            let height = agent_target(&ctx, "status-sidebar-height").expect("sidebar height");
            assert!(
                !resources.intersects(toggle),
                "resources {resources:?} overlap terminal toggle {toggle:?} (collapsed={collapsed})"
            );
            assert!(
                !resources.intersects(height) && !height.intersects(toggle),
                "sidebar height {height:?} overlaps resources {resources:?} or toggle {toggle:?}"
            );
            // Terminal button keeps the far-right corner. Sidebar height sits
            // immediately left of it, and the readout sits left of that.
            assert!(
                height.right() <= toggle.left() && resources.right() <= height.left(),
                "resources {resources:?} height {height:?} toggle {toggle:?}"
            );
        }
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn status_sidebar_height_toggles_the_preference() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.ide_mode = true;
        assert!(app.preferences.ide_sidebars_full_height);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 40.0),
                )),
                ..Default::default()
            },
            |ui| app.status_right_end(ui),
        );
        output.textures_delta.clear();
        let height = agent_target(&ctx, "status-sidebar-height").expect("sidebar height");
        let pos = height.center();
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 40.0),
                    )),
                    events: vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    }],
                    ..Default::default()
                },
                |ui| app.status_right_end(ui),
            );
            output.textures_delta.clear();
        }
        assert!(!app.preferences.ide_sidebars_full_height);
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
    #[cfg(feature = "test-support")]
    fn header_paints_a_sidebar_toggle_on_each_side() {
        let (mut app, ctx, _dir) = fixture();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 40.0),
                )),
                ..Default::default()
            },
            |ui| app.window_header(ui),
        );
        output.textures_delta.clear();
        let rect = |name: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
        };
        let left = rect("toggle-left-sidebar").expect("left toggle");
        let right = rect("toggle-right-sidebar").expect("right toggle");
        assert!(left.right() < right.left());
        assert!(left.width() > 0.0 && right.width() > 0.0);
        app.preferences.left_visible = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 40.0),
                )),
                ..Default::default()
            },
            |ui| app.window_header(ui),
        );
        output.textures_delta.clear();
        assert!(rect("toggle-left-sidebar").is_some());
        assert!(rect("toggle-right-sidebar").is_some());
    }
}
