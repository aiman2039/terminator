use super::super::appearance;
use eframe::egui::{self};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    /// Info paints the focused session and host meters, and hides a block when
    /// its section or the SYSTEM toggle is closed. A sample for another pid
    /// does not fill THIS SESSION.
    #[test]
    #[cfg(feature = "test-support")]
    fn info_sidebar_paints_process_and_system_meters() {
        let (mut app, _ctx, _dir) = fixture();
        let mut session = session_fixture("term-3", SessionKind::Shell);
        session.label = "Terminal 3".into();
        session.cwd = "/Users/ohaddahan/RustroverProjects/terminator".into();
        session.created = now().saturating_sub(120);
        session.pid = Some(42);
        session.lifecycle = Lifecycle::Running;
        app.active_session = Some(session.id.clone());
        app.selected = Some("a".into());
        app.metadata = Some(metadata::Metadata {
            cwd: session.cwd.clone(),
            branch: Some("master".into()),
            ..Default::default()
        });
        app.state.sessions = vec![session.clone()];
        app.preferences.visible = true;
        app.preferences.tool = SidebarTool::Info;
        app.preferences.info_process_open = true;
        app.preferences.info_resources_open = true;
        app.preferences.info_show_system = true;
        let system = resource_sample::SystemStats {
            cpu: 29.0,
            memory_used: 439_u64
                .saturating_mul(1024)
                .saturating_mul(1024)
                .saturating_mul(1024)
                .checked_div(10)
                .unwrap_or(0),
            memory_total: 128 * 1024 * 1024 * 1024,
            pressure: Some(terminator_sys::MemoryPressure {
                percent: 10.0,
                level: terminator_sys::PressureLevel::Normal,
            }),
            load_one: 6.70,
            load_five: 5.43,
            load_fifteen: 6.15,
            cpus: 10,
        };
        app.resources = Some(resource_sample::Sample {
            pid: Some(42),
            started: session.created,
            session: None,
            system: system.clone(),
            app: resource_sample::AppStats::default(),
        });

        fn painted(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
            fn walk(out: &mut Vec<String>, shape: &egui::Shape) {
                match shape {
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            walk(out, shape);
                        }
                    }
                    egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in shapes {
                walk(&mut out, &clipped.shape);
            }
            out
        }
        let paint = |app: &mut App| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(420.0, 720.0),
                    )),
                    ..Default::default()
                },
                |ui| app.sidebar(ui),
            );
            let text = painted(&output.shapes);
            output.textures_delta.clear();
            (text, ctx)
        };
        let contains =
            |text: &[String], needle: &str| text.iter().any(|line| line.contains(needle));
        let marked = |ctx: &egui::Context, name: &str| {
            ctx.data(|data| {
                data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name)))
                    .is_some()
            })
        };

        let (text, ctx) = paint(&mut app);
        for name in [
            session_info::SESSION,
            session_info::CWD,
            session_info::BRANCH,
            session_info::STARTED,
            session_info::SESSION_CPU,
            session_info::SESSION_MEMORY,
            session_info::SYSTEM_CPU,
            session_info::SYSTEM_MEMORY,
            session_info::PRESSURE,
            session_info::LOAD,
            session_info::SYSTEM_TOGGLE,
        ] {
            assert!(marked(&ctx, name), "missing {name}");
        }
        assert!(contains(&text, "Terminal 3"));
        assert!(contains(&text, "RustroverProjects"));
        assert!(contains(&text, "master"));
        assert!(contains(&text, "2m ago"));
        assert!(contains(&text, "29%"));
        assert!(contains(&text, "128.0 GB"));
        assert!(contains(&text, "10% · normal"));
        assert!(contains(&text, "6.70 5.43 6.15"));
        assert!(contains(&text, "THIS SESSION"));

        app.resources = Some(resource_sample::Sample {
            pid: Some(99),
            started: session.created,
            session: Some(resource_sample::SessionStats {
                cpu: 12.0,
                memory: 50 * 1024 * 1024,
            }),
            system: system.clone(),
            app: resource_sample::AppStats::default(),
        });
        let (text, ctx) = paint(&mut app);
        assert!(marked(&ctx, session_info::SESSION_CPU));
        assert!(!contains(&text, "12%"));
        assert!(!contains(&text, "50 MB"));
        assert!(contains(&text, "29%"));

        app.preferences.info_show_system = false;
        let (text, ctx) = paint(&mut app);
        assert!(!marked(&ctx, session_info::LOAD));
        assert!(marked(&ctx, session_info::SESSION));
        assert!(contains(&text, "Terminal 3"));
        assert!(!contains(&text, "6.70 5.43 6.15"));

        app.preferences.info_process_open = false;
        app.preferences.info_show_system = true;
        let (text, ctx) = paint(&mut app);
        assert!(!marked(&ctx, session_info::SESSION));
        assert!(marked(&ctx, session_info::LOAD));
        assert!(!contains(&text, "Terminal 3"));
        assert!(contains(&text, "6.70 5.43 6.15"));
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn player_chrome_paints_next_to_the_project_bell() {
        let (mut app, ctx, _dir) = fixture();
        app.project_width = 420.0;
        if let Some(project) = app
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == "a")
        {
            project.name = "ai-proxy".into();
        }
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        let name = rect("project-header-name").expect("project name");
        let chrome = rect("player-chrome").expect("player chrome");
        let bell = rect("left-agent-bar").expect("project bell");
        let toggle = rect("toggle-left-sidebar").expect("hide sidebar");
        let controls = rect("window-controls").expect("window controls");
        assert!(rect("project-header-menu").is_none());
        assert!(
            name.left() >= controls.right() - 1.0 && name.left() - controls.right() < 16.0,
            "the name hugs the left, after the traffic lights, name={name:?} controls={controls:?}"
        );
        assert!(
            chrome.left() - name.right() > 8.0,
            "spare width sits between the name and the player, name={name:?} chrome={chrome:?}"
        );
        assert!(
            chrome.right() <= bell.left() + 1.0 && bell.left() - chrome.right() < 8.0,
            "player sits against the bell, chrome={chrome:?} bell={bell:?}"
        );
        assert!(
            bell.right() <= toggle.left() + 1.0 && toggle.left() - bell.right() < 8.0,
            "bell sits against hide, bell={bell:?} toggle={toggle:?}"
        );
        assert!(
            (name.center().y - toggle.center().y).abs() < 2.0,
            "name and hide share a row, name={name:?} toggle={toggle:?}"
        );
        assert!(
            toggle.right() > app.project_width - 16.0,
            "the group is packed to the sidebar's right edge, toggle={toggle:?}"
        );
        assert!(chrome.height() <= 28.0 && bell.height() <= 28.0);
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn default_sidebar_keeps_player_and_bell_beside_hide() {
        let (mut app, ctx, _dir) = fixture();
        app.project_width = 225.0;
        if let Some(project) = app
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == "a")
        {
            project.name = "ai-proxy".into();
        }
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        let controls = rect("window-controls").expect("window controls");
        assert!(
            (controls.width() - 72.0).abs() < 0.5,
            "window controls use the 72px slot, controls={controls:?}"
        );
        assert!(
            rect("player-chrome").is_some() && rect("left-agent-bar").is_some(),
            "a default sidebar fits the player and the bell, menu={:?}",
            rect("project-header-menu")
        );
        assert!(rect("project-header-menu").is_none());
        if let Some(project) = app
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == "a")
        {
            project.name = "other-project".into();
        }
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        assert!(
            rect("left-agent-bar").is_some(),
            "the agents fixture name still paints the bell, menu={:?}",
            rect("project-header-menu")
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn narrow_project_header_puts_player_and_bell_in_the_menu() {
        let (mut app, ctx, _dir) = fixture();
        app.project_width = 120.0;
        if let Some(project) = app
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == "a")
        {
            project.name = "ai-proxy".into();
        }
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        assert!(rect("project-header-menu").is_some());
        assert!(rect("player-chrome").is_none());
        assert!(rect("left-agent-bar").is_none());
        assert!(rect("project-header-name").is_some());
        let menu = rect("project-header-menu").expect("menu");
        let pos = menu.center();
        paint_header(&mut app, &ctx, &[egui::Event::PointerMoved(pos)]);
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        assert!(rect("Player").is_some(), "menu offers Player");
        assert!(rect("Agents").is_some(), "menu offers Agents");
    }

    #[test]
    fn older_observation_cannot_restore_a_previous_active_generation() {
        let (mut app, _, _directory) = fixture();
        let mut state = app.state.clone();
        state.generation = "new-owner".into();
        state.client_observation = 3;
        app.apply_state(state.clone());
        state.generation = "old-owner".into();
        state.client_observation = 2;
        app.apply_state(state);
        assert_eq!(app.state.generation, "new-owner");
    }

    #[test]
    fn shell_exit_is_applied_when_a_newer_revision_has_an_older_observation() {
        let (mut app, _, _directory) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.insert("a", Tab::Terminal("shell".into()), None);
        app.state.generation = "owner".into();
        app.state.revision = 10;
        app.state.client_observation = 5;
        let mut ended = app.state.clone();
        ended.client_observation = 4;
        ended.revision = 11;
        ended
            .sessions
            .iter_mut()
            .find(|session| session.id == "shell")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(ended);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("shell".into()))
                .is_none()
        );
        assert_eq!(app.state.revision, 11);
    }

    #[test]
    fn stale_historical_owner_does_not_block_shell_exit_cleanup() {
        let (mut app, _, directory) = fixture();
        let mut shell = session_fixture("shell", SessionKind::Shell);
        shell.generation = "active".into();
        app.state.sessions.push(shell);
        app.insert("a", Tab::Terminal("shell".into()), None);
        app.state.generation = "active".into();
        app.state.revision = 10;
        app.state.client_observation = 5;
        app.state.generations = vec![
            generation_health("active", directory.path(), 10, generations::Status::Active),
            generation_health(
                "retired",
                directory.path(),
                50,
                generations::Status::Retired,
            ),
        ];
        let mut ended = app.state.clone();
        ended.revision = 11;
        ended.client_observation = 6;
        ended.generations[0].revision = 11;
        ended.generations[1].revision = 49;
        ended
            .sessions
            .iter_mut()
            .find(|session| session.id == "shell")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(ended);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("shell".into()))
                .is_none()
        );
    }

    #[test]
    fn poller_ended_snapshot_wins_over_an_older_job_snapshot() {
        let (mut app, ctx, _directory) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.insert("a", Tab::Terminal("shell".into()), None);
        app.state.generation = "owner".into();
        app.state.revision = 10;
        app.state.client_observation = 1;
        let mut stale = app.state.clone();
        stale.client_observation = 3;
        let mut ended = app.state.clone();
        ended.client_observation = 2;
        ended.revision = 11;
        ended
            .sessions
            .iter_mut()
            .find(|session| session.id == "shell")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.service_ready.push_back(Update::State(Box::new(stale)));
        *app.service_owner.snapshot.lock().unwrap() = Some(Box::new(ended));
        app.process_updates(&ctx);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("shell".into()))
                .is_none()
        );
    }

    #[test]
    fn unavailable_owner_keeps_last_records_but_updates_health() {
        let (mut app, _, directory) = fixture();
        let mut state = app.state.clone();
        state.generation = "owner".into();
        state.revision = 20;
        let mut session = session_fixture("last-observed", SessionKind::Shell);
        session.generation = "owner".into();
        state.sessions.push(session);
        state.generations.push(generations::Health {
            owner: generations::Generation {
                id: "owner".into(),
                data: directory.path().into(),
                runtime: directory.path().into(),
                version: "0.35.0".into(),
                build: "fixture".into(),
                protocol: 1,
                catalog: 1,
                status: generations::Status::Active,
                pid: None,
            },
            revision: 20,
            error: None,
            live_sessions: 1,
            capabilities: Vec::new(),
            helper: None,
        });
        app.apply_state(state.clone());
        state.sessions.clear();
        state.revision = 19;
        state.generations[0].revision = 19;
        state.generations[0].error = Some("Owner unavailable".into());
        app.apply_state(state);
        assert!(
            app.state
                .sessions
                .iter()
                .any(|session| session.id == "last-observed")
        );
        assert!(app.state.generations[0].error.is_some());
    }

    #[test]
    fn late_snapshot_cannot_erase_a_newer_project_inventory() {
        let (mut app, _, _dir) = fixture();
        let mut fresh = app.state.clone();
        fresh.generation = "snapshot-owner".into();
        fresh.revision = 20;
        fresh.catalog_revision = 7;
        let projects = fresh.projects.len();
        assert!(projects > 0);
        app.apply_state(fresh.clone());
        let mut stale = fresh;
        stale.revision = 19;
        stale.projects.clear();
        app.apply_state(stale);
        assert_eq!(app.state.projects.len(), projects);
        assert_eq!(app.state.revision, 20);
    }

    #[test]
    fn opening_player_seeds_sample_tracks_once() {
        let (mut app, _ctx, _dir) = fixture();
        app.open_player();
        assert_eq!(app.preferences.selected_tracks().len(), 3);
        assert!(
            app.preferences
                .selected_tracks()
                .iter()
                .all(|path| path.extension().is_some_and(|ext| ext == "wav"))
        );
        app.preferences
            .selected_tracks_mut()
            .expect("playlist")
            .clear();
        app.open_player();
        assert!(app.preferences.selected_tracks().is_empty());
    }

    #[test]
    fn player_icon_opens_a_global_window_not_a_tab() {
        let (mut app, _, _dir) = fixture();
        app.open_player();
        assert!(app.player_open);
        assert!(!app.layouts["a"].contains(&Tab::Player));
        app.open_player();
        assert!(app.player_open);
        assert_eq!(
            app.layouts["a"]
                .iter_all_tabs()
                .filter(|(_, tab)| matches!(tab, Tab::Player))
                .count(),
            0
        );
    }

    #[test]
    fn leftover_player_tabs_are_stripped_without_stopping_playback() {
        let (mut app, _, _dir) = fixture();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("player-a".into(), Tab::Player);
        app.player = player::Controller::finished_fixture("a", Some(0));
        app.reconcile_gui_resources();
        assert!(!app.layouts["a"].contains(&Tab::Player));
        assert_eq!(app.player.project.as_deref(), Some("a"));
    }

    #[test]
    fn opening_the_player_closes_a_leftover_player_tab() {
        let (mut app, _, _dir) = fixture();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("player-a".into(), Tab::Player);
        app.open_player();
        assert!(app.player_open);
        assert!(!app.layouts["a"].contains(&Tab::Player));
        app.open_player();
        assert!(app.player_open);
    }

    #[test]
    fn settings_and_player_are_center_singletons() {
        let (mut app, _, _dir) = fixture();
        app.open_settings();
        app.settings_draft.shell = "/tmp/custom-shell".into();
        app.settings_section = SettingsSection::Terminal;
        app.settings_search = "shell".into();
        app.hide_center_overlay();
        assert!(!app.settings_open);
        assert!(app.settings_session);
        app.open_settings();
        assert_eq!(app.settings_draft.shell, "/tmp/custom-shell");
        assert_eq!(app.settings_section, SettingsSection::Terminal);
        assert_eq!(app.settings_search, "shell");
        app.open_player();
        assert!(app.player_open);
        assert!(!app.settings_open);
        assert!(app.settings_session);
        app.open_settings();
        assert!(!app.player_open);
        assert_eq!(app.settings_draft.shell, "/tmp/custom-shell");
        app.end_settings_session();
        app.open_settings();
        assert!(app.settings_draft.shell.is_empty());
        assert!(app.settings_search.is_empty());
    }

    #[test]
    fn go_session_hides_center_overlay() {
        let (mut app, _, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.open_player();
        app.go_session("shell");
        assert!(!app.player_open);
        app.open_player();
        assert!(app.player_open);
    }

    #[test]
    fn create_hides_settings_without_ending_session() {
        let (mut app, _, _dir) = fixture();
        app.open_settings();
        app.settings_search = "shell".into();
        app.create(None);
        assert!(!app.settings_open);
        assert!(app.settings_session);
        app.open_settings();
        assert_eq!(app.settings_search, "shell");
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn attention_actions_fit_minimum_sidebar_widths() {
        for width in [170.0, 220.0, 320.0] {
            let (mut app, ctx, _dir) = fixture();
            appearance::install(&ctx);
            app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
            app.state.agents = vec![Agent {
                invocation_id: "agent".into(),
                session_id: "live-shell".into(),
                kind: "codex".into(),
                provider_session_id: None,
                state: AgentState::WaitingPermission,
                sequence: None,
                updated: 0,
                resume: None,
                process: None,
            }];
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
                        egui::vec2(width, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let bounds = ui.max_rect();
                    app.agents_view(ui);
                    let mut action_rects = Vec::new();
                    for action in ["go", "snooze", "dismiss"] {
                        let rect =
                            agent_target(&ctx, &format!("agent-{action}:live-shell")).unwrap();
                        assert!(
                            bounds.contains_rect(rect),
                            "width {width}: {action} {rect:?} outside {bounds:?}"
                        );
                        action_rects.push(rect);
                    }
                    assert!(
                        (action_rects[0].center().y - action_rects[2].center().y).abs() < 2.0,
                        "width {width}: actions should stay one cluster {action_rects:?}"
                    );
                    let row = agent_target(&ctx, "agent-row:live-shell").unwrap();
                    assert!(
                        (row.center().y - action_rects[0].center().y).abs() < 8.0,
                        "width {width}: title and actions should share one row"
                    );
                    assert!(
                        action_rects[0].min.x >= row.max.x - 2.0,
                        "width {width}: actions should follow the title"
                    );
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agent_tabs_share_one_row() {
        let (mut app, ctx, _dir) = fixture();
        appearance::install(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.agents_view(ui);
            },
        );
        output.textures_delta.clear();
        let tabs: Vec<_> = ["needs", "live", "unread"]
            .into_iter()
            .map(|name| agent_target(&ctx, &format!("agent-tab:{name}")).unwrap())
            .collect();
        for tab in &tabs[1..] {
            assert!(
                (tab.center().y - tabs[0].center().y).abs() < 1.0,
                "tab centers should share one row: {tabs:?}"
            );
        }
    }
}
