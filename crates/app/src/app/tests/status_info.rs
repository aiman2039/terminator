use super::*;

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
        painted_strings(&output.shapes)
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
        let text = painted_strings(&output.shapes);
        output.textures_delta.clear();
        (text, ctx)
    };
    let contains = |text: &[String], needle: &str| text.iter().any(|line| line.contains(needle));
    let marked = |ctx: &egui::Context, name: &str| {
        ctx.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name)))
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
fn system_meter_bars_share_one_column() {
    let (app, ctx, _dir) = fixture();
    appearance::install(&ctx);
    let input = session_info::Input {
        label: Some("Terminal 3".into()),
        cwd: Some("~/proj".into()),
        cwd_full: Some("/Users/x/proj".into()),
        branch: Some("master".into()),
        started: "2m ago".into(),
        unconfirmed: false,
        session_cpu: Some(1.9),
        session_memory: Some(357 * 1024 * 1024),
        system: Some(resource_sample::SystemStats {
            cpu: 14.0,
            memory_used: 555_u64
                .saturating_mul(1024)
                .saturating_mul(1024)
                .saturating_mul(1024)
                .checked_div(10)
                .unwrap_or(0),
            memory_total: 128 * 1024 * 1024 * 1024,
            pressure: Some(terminator_sys::MemoryPressure {
                percent: 0.0,
                level: terminator_sys::PressureLevel::Normal,
            }),
            load_one: 2.89,
            load_five: 2.98,
            load_fifteen: 3.17,
            cpus: 10,
        }),
    };
    let model = session_info::model(&input);
    let mut toggles = session_info::Toggles {
        process_open: false,
        resources_open: true,
        show_system: true,
    };
    let theme = app.theme.clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            session_info::show(ui, &model, &mut toggles, &theme);
        },
    );
    output.textures_delta.clear();
    let bars: Vec<_> = [
        "info-system-cpu-bar",
        "info-system-memory-bar",
        "info-pressure-bar",
        "info-load-bar",
    ]
    .into_iter()
    .map(|name| agent_target(&ctx, name).unwrap())
    .collect();
    for bar in &bars[1..] {
        assert!(
            (bar.min.x - bars[0].min.x).abs() < 1.0,
            "meter bars should share one column: {bars:?}"
        );
    }
}
