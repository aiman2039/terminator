use super::*;

#[test]
#[cfg(feature = "test-support")]
fn sidebar_group_is_abstract_and_renames_without_touching_sessions() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions = vec![
        session_fixture("s1", SessionKind::Shell),
        session_fixture("s2", SessionKind::Shell),
        session_fixture("s3", SessionKind::Shell),
    ];
    let tab = |id: &str, sids: &[&str]| crate::workspace::WorkspaceTab {
        id: id.into(),
        name: None,
        pinned: false,
        primary: Some(Tab::Terminal(sids[0].into())),
        layout: egui_dock::DockState::new(
            sids.iter()
                .map(|sid| Tab::Terminal((*sid).to_string()))
                .collect(),
        ),
    };
    let mut workspace =
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal("s1".into())]));
    // Two terminals so the tab keeps a node to rename.
    workspace.tabs[0] = tab("g1", &["s1", "s3"]);
    workspace.tabs.push(tab("g2", &["s2"]));
    workspace.active = "g1".into();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());

    let mut time = 0.0_f64;
    let paint = |app: &mut App, events: &[egui::Event], time: &mut f64| {
        *time += 0.05;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 600.0),
                )),
                time: Some(*time),
                events: events.to_vec(),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
    };
    paint(&mut app, &[], &mut time);
    paint(&mut app, &[], &mut time);
    let header = header_target(&ctx)("sidebar-tab:g1").expect("group header");
    let pos = header.center();
    let press = |pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    // Double-click starts the rename; the group then owns its own text.
    paint(&mut app, &[press(true)], &mut time);
    paint(&mut app, &[press(false)], &mut time);
    paint(&mut app, &[press(true)], &mut time);
    paint(&mut app, &[press(false)], &mut time);
    paint(&mut app, &[], &mut time);
    assert_eq!(app.rename_group, Some(("a".to_string(), "g1".to_string())));
    assert!(header_target(&ctx)("sidebar-tab-rename:g1").is_some());
    paint(&mut app, &[egui::Event::Text("Agents".into())], &mut time);
    paint(&mut app, &[key_press(egui::Key::Enter)], &mut time);
    assert_eq!(app.rename_group, None);
    assert_eq!(
        app.layouts["a"].tabs[0].name.as_deref(),
        Some("Agents"),
        "group keeps its own name"
    );
    // Member terminal labels are untouched by the group rename.
    assert_eq!(app.state.sessions[0].label, "s1");
    // Renaming again starts from the custom name.
    paint(&mut app, &[press(true)], &mut time);
    paint(&mut app, &[press(false)], &mut time);
    paint(&mut app, &[press(true)], &mut time);
    paint(&mut app, &[press(false)], &mut time);
    assert_eq!(app.rename_group_text, "Agents");
    paint(&mut app, &[key_press(egui::Key::Escape)], &mut time);
    assert_eq!(app.rename_group, None);
    assert_eq!(app.layouts["a"].tabs[0].name.as_deref(), Some("Agents"));
}

#[test]
fn sidebar_models_reuse_data_until_snapshot_or_filter_changes() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.reconcile_presentations();
    let index = app.sidebar_index();
    let projects = app.cached_projects();
    let history = app.cached_history();
    let live = app.cached_live_rows();
    app.active_session = Some("shell".into());
    app.preferences.expanded.insert("a".into(), false);
    assert!(std::sync::Arc::ptr_eq(&index, &app.sidebar_index()));
    assert!(std::sync::Arc::ptr_eq(&projects, &app.cached_projects()));
    assert!(std::sync::Arc::ptr_eq(&history, &app.cached_history()));
    assert!(std::sync::Arc::ptr_eq(&live, &app.cached_live_rows()));
    app.preferences.history_filter = "changed".into();
    assert!(!std::sync::Arc::ptr_eq(&history, &app.cached_history()));
    app.preferences.hidden_projects.insert("a".into());
    assert_eq!(
        app.cached_projects()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
    let mut state = app.state.clone();
    state.sessions[0].label = "renamed".into();
    app.apply_state(state);
    let next = app.sidebar_index();
    assert!(!std::sync::Arc::ptr_eq(&index, &next));
    assert_eq!(next.sessions["shell"].label, "renamed");
}

#[test]
fn sidebar_notice_cache_updates_on_read_dismiss_and_snooze_expiry() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .notifications
        .push(notice_fixture(NoticeFixtureInput {
            id: "notice",
            session: "shell",
            state: AgentState::WaitingInput,
            created: 1,
        }));
    app.reconcile_presentations();
    assert_eq!(app.attention_counts(), (1, 1));
    let unread = app.cached_unread_groups();
    app.apply_notice_action("notice".into(), AttentionAction::Read);
    assert_eq!(app.attention_counts(), (1, 0));
    assert_eq!(app.cached_unread_groups().len(), 1);
    assert!(!std::sync::Arc::ptr_eq(
        &unread,
        &app.cached_unread_groups()
    ));
    app.unread_selected = None;
    assert!(app.cached_unread_groups().is_empty());
    app.apply_notice_action("notice".into(), AttentionAction::Snooze);
    assert_eq!(app.attention_counts(), (0, 0));
    let deadline = app.state.notifications[0].snoozed_until;
    assert!(app.sidebar_index_at(deadline - 1).pending.is_empty());
    assert_eq!(app.sidebar_index_at(deadline).pending.len(), 1);
    app.apply_notice_action("notice".into(), AttentionAction::Dismiss);
    assert!(app.sidebar_index().pending.is_empty());
}

#[test]
fn project_sidebar_sorts_by_name_and_latest_activity() {
    let (mut app, _, _) = fixture();
    app.state.projects[0].name = "zeta".into();
    app.state.projects[1].name = "alpha".into();
    assert_eq!(visible_ids(&app), ["b", "a"]);
    app.preferences.project_sort = ProjectSort::NameDesc;
    assert_eq!(visible_ids(&app), ["a", "b"]);
    app.preferences.project_sort = ProjectSort::LatestActivity;
    app.preferences.project_activity.insert("a".into(), 1);
    app.preferences.project_activity.insert("b".into(), 2);
    assert_eq!(visible_ids(&app), ["b", "a"]);
    app.select_project("a".into());
    assert_eq!(visible_ids(&app), ["b", "a"]);
    assert_eq!(app.preferences.project_activity["a"], 1);
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.go_session("shell");
    assert_eq!(visible_ids(&app), ["b", "a"]);
    assert_eq!(app.preferences.project_activity["a"], 1);
}

#[test]
fn restoring_a_hidden_project_ranks_it_by_latest_activity() {
    let (mut app, _, _) = fixture();
    app.preferences.project_sort = ProjectSort::LatestActivity;
    app.preferences.project_activity.insert("a".into(), 1);
    app.preferences.project_activity.insert("b".into(), 2);
    app.hide_project("a");
    assert_eq!(visible_ids(&app), ["b"]);
    assert_eq!(app.preferences.project_activity["b"], 2);
    app.select_project("a".into());
    assert_eq!(visible_ids(&app), ["a", "b"]);
    assert!(app.preferences.project_activity["a"] >= 2);
    assert_eq!(app.preferences.project_activity["b"], 2);
}

#[test]
fn opening_or_creating_a_project_ranks_it_by_latest_activity() {
    let (mut app, ctx, _) = fixture();
    app.preferences.project_sort = ProjectSort::LatestActivity;
    app.preferences.project_activity.insert("a".into(), 1);
    app.preferences.project_activity.insert("b".into(), 2);
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "a".into(),
            app.selection_generation,
        ))
        .unwrap();
    drain_until_project_activity(DrainUntilProjectActivityInput {
        app: &mut app,
        ctx: &ctx,
        project: "a",
        at_least: 2,
    });
    assert_eq!(visible_ids(&app), ["a", "b"]);
    assert!(app.preferences.project_activity["a"] >= 2);
    app.preferences.project_activity.insert("a".into(), 1);
    app.update_tx
        .send(Update::WorktreeCreated(
            Box::new(app.state.clone()),
            "a".into(),
            false,
        ))
        .unwrap();
    drain_until_project_activity(DrainUntilProjectActivityInput {
        app: &mut app,
        ctx: &ctx,
        project: "a",
        at_least: 2,
    });
    assert_eq!(visible_ids(&app), ["a", "b"]);
    assert!(app.preferences.project_activity["a"] >= 2);
}

#[test]
fn hiding_the_selected_project_selects_the_next_sorted_project() {
    let (mut app, _, _) = fixture();
    app.state.projects.push(Project {
        id: "c".into(),
        name: "alpha".into(),
        path: "/c".into(),
        layout: serde_json::Value::Null,
    });
    app.state.projects[0].name = "zeta".into();
    app.state.projects[1].name = "mu".into();
    app.selected = Some("a".into());
    app.hide_project("a");
    assert_eq!(app.selected.as_deref(), Some("c"));
    assert_eq!(visible_ids(&app), ["c", "b"]);
}

#[test]
fn removing_projects_only_hides_sidebar_entries_and_survives_snapshots() {
    let (mut app, ctx, dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    let layouts = serde_json::to_value(&app.layouts).unwrap();
    let sessions = serde_json::to_value(&app.state.sessions).unwrap();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.hide_project("a");
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(app.preferences.hidden_projects.contains("a"));
    assert!(requests.try_iter().all(|job| matches!(job, Job::Control(request, _) if matches!(*request, Request::SelectProject { .. }))));
    app.hide_project("b");
    assert!(app.selected.is_none());
    app.apply_state(app.state.clone());
    assert!(app.selected.is_none());
    assert_eq!(serde_json::to_value(&app.layouts).unwrap(), layouts);
    assert_eq!(serde_json::to_value(&app.state.sessions).unwrap(), sessions);
    assert_eq!(app.state.projects.len(), 2);
    app.preferences.save(dir.path()).unwrap();
    assert_eq!(
        UiPreferences::load(dir.path())
            .unwrap()
            .hidden_projects
            .len(),
        2
    );
    // Adding the same folder again restores its existing ID and layout.
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "a".into(),
            app.selection_generation,
        ))
        .unwrap();
    pump_until(&mut app, &ctx, |app| app.selected.as_deref() == Some("a"));
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert!(!app.preferences.hidden_projects.contains("a"));
    assert_eq!(serde_json::to_value(&app.layouts).unwrap(), layouts);
    assert_eq!(serde_json::to_value(&app.state.sessions).unwrap(), sessions);
}

#[test]
fn a_delayed_folder_open_does_not_restore_a_project_removed_afterward() {
    let (mut app, ctx, _dir) = fixture();
    let generation = app.selection_generation;
    app.hide_project("a");
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "a".into(),
            generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.preferences.hidden_projects.contains("a"));
    assert_eq!(app.selected.as_deref(), Some("b"));
}

#[test]
#[cfg(feature = "test-support")]
fn expanded_project_terminals_stay_indented_at_all_sidebar_sizes() {
    for width in [180.0, 280.0, 420.0] {
        for scale in [1.0, 2.0] {
            let (mut app, ctx, _dir) = fixture();
            ctx.set_pixels_per_point(scale);
            let first = session_fixture("first-shell", SessionKind::Shell);
            let mut second = session_fixture("second-shell", SessionKind::Shell);
            second.project_id = "b".into();
            app.state.sessions = vec![first, second];
            for project in ["a", "b"] {
                app.preferences.expanded.insert(project.into(), true);
            }
            for selected in ["a", "b"] {
                app.selected = Some(selected.into());
                // Include the first frame and settled layout frames.
                for _ in 0..3 {
                    let expected_indent = paint_projects_sidebar(&mut app, &ctx, width);
                    assert_session_rows_indented(
                        &ctx,
                        expected_indent,
                        &format!("width={width}, scale={scale}"),
                    );
                }
            }
        }
    }
}

/// Paints the projects sidebar `width` wide; returns egui's indent.
#[cfg(feature = "test-support")]
fn paint_projects_sidebar(app: &mut App, ctx: &egui::Context, width: f32) -> f32 {
    let mut expected_indent = 0.0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            ..Default::default()
        },
        |ui| {
            expected_indent = ui.spacing().indent;
            app.projects(ui);
        },
    );
    output.textures_delta.clear();
    expected_indent
}

#[cfg(feature = "test-support")]
fn assert_session_rows_indented(ctx: &egui::Context, expected_indent: f32, label: &str) {
    let target = |name: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
            .unwrap()
    };
    for (project, session) in [("a", "first-shell"), ("b", "second-shell")] {
        let parent = target(&format!("project-row:{project}"));
        let child = target(&format!("session-row:{session}"));
        let indent = child.left() - parent.left();
        assert!(
            indent >= expected_indent - 0.1,
            "{label}, project={project}: indent={indent}"
        );
        assert!(child.top() >= parent.bottom());
    }
}

#[test]
#[cfg(feature = "test-support")]
fn project_list_excludes_editors_and_global_history_includes_other_projects() {
    let (mut app, ctx, _dir) = fixture();
    let mut ended = session_fixture("ended-other", SessionKind::Shell);
    ended.project_id = "b".into();
    ended.lifecycle = Lifecycle::Ended;
    let mut resumable = session_fixture("ended-resumable", SessionKind::Shell);
    resumable.project_id = "b".into();
    resumable.lifecycle = Lifecycle::Ended;
    app.state.sessions = vec![
        session_fixture("live-shell", SessionKind::Shell),
        session_fixture("open-file", SessionKind::Editor),
        ended,
        resumable,
    ];
    app.state.agents = vec![Agent {
        invocation_id: "agent-1".into(),
        session_id: "ended-resumable".into(),
        kind: "codex".into(),
        provider_session_id: Some("provider-1".into()),
        state: AgentState::Stopped,
        sequence: None,
        updated: 0,
        resume: Some(Resume {
            program: "codex".into(),
            args: vec!["resume".into(), "provider-1".into()],
        }),
        process: None,
    }];
    app.preferences.expanded.insert("a".into(), true);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.projects(ui));
    output.textures_delta.clear();
    let target = |name: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
    };
    assert!(target("session-row:live-shell").is_some());
    assert!(target("session-row:open-file").is_none());
    assert!(target("session-row:ended-other").is_none());
    app.preferences.tool = SidebarTool::History;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.sidebar(ui));
    output.textures_delta.clear();
    assert!(target("session-row:ended-other").is_none());
    assert!(target("session-row:ended-resumable").is_some());
    assert!(target("history-filter").is_some());
    assert!(target("history-sort").is_some());
    assert!(target("history-toggle-all").is_some());
    assert_eq!(app.state.sessions.len(), 4);
}

#[test]
fn project_sidebar_does_not_repeat_the_live_session_count() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(280.0, 500.0),
            )),
            ..Default::default()
        },
        |ui| app.projects(ui),
    );
    let text = painted_strings(&output.shapes);
    output.textures_delta.clear();
    assert!(
        text.iter().all(|line| !line.contains("live sessions")),
        "{text:?}"
    );
}

#[test]
fn project_sidebar_paints_a_working_agent_icon() {
    let (mut app, ctx, _dir) = fixture();
    appearance::install(&ctx);
    app.selected = Some("a".into());
    app.preferences.expanded.insert("a".into(), true);
    app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
    app.state.agents = vec![Agent {
        invocation_id: "agent".into(),
        session_id: "s".into(),
        kind: "codex".into(),
        provider_session_id: None,
        state: AgentState::Running,
        sequence: None,
        updated: 1,
        resume: None,
        process: None,
    }];
    let presented = app.present_session("s");
    assert_eq!(presented.brand_icon, Some("AgentCodex"));
    assert_eq!(presented.status_icon, "LoaderCircle");
    let mut marks = 0;
    for _ in 0..4 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        marks = output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(
                    &shape.shape,
                    egui::Shape::Rect(rect) if rect.brush.is_some()
                ) || matches!(&shape.shape, egui::Shape::Mesh(_))
            })
            .count();
        output.textures_delta.clear();
    }
    assert!(
        marks >= 2,
        "a working agent must paint its brand icon and status icon, got {marks}"
    );
}

#[test]
#[cfg(feature = "test-support")]
fn project_header_controls_share_height() {
    let (mut app, ctx, _dir) = fixture();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            ..Default::default()
        },
        |ui| app.projects(ui),
    );
    output.textures_delta.clear();
    let target = |name: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
            .unwrap()
    };
    let add = target("project-add");
    let sort = target("project-sort");
    assert!(
        (add.height() - sort.height()).abs() < 8.0,
        "add={add:?} sort={sort:?}"
    );
    assert!(
        (add.center().y - sort.center().y).abs() < 4.0,
        "add={add:?} sort={sort:?}"
    );
    let filter = target("project-filter");
    assert!(
        (filter.height() - add.height()).abs() < 8.0,
        "filter={filter:?} add={add:?}"
    );
    assert!(
        (filter.center().y - add.center().y).abs() < 4.0,
        "filter={filter:?} add={add:?}"
    );
}

#[test]
#[cfg(feature = "test-support")]
fn project_filter_narrows_visible_projects() {
    let (mut app, ctx, _dir) = fixture();
    assert_eq!(app.cached_projects().len(), 2);
    app.preferences.project_filter = "b".into();
    let visible = app.cached_projects();
    assert_eq!(
        visible.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["b"]
    );
    app.preferences.project_filter = "zzz".into();
    assert!(app.cached_projects().is_empty());
    app.preferences.project_filter.clear();
    assert_eq!(app.cached_projects().len(), 2);
    let _ = &ctx;
}

#[test]
#[cfg(feature = "test-support")]
fn project_filter_field_owns_text_keys() {
    let (mut app, ctx, _dir) = fixture();
    assert!(!app.text_input_focused(&ctx));
    let paint = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
    };
    paint(&mut app);
    // The filter ID is scoped to its hosting sidebar, so focus it the
    // way a user does: click the painted field.
    let rect = ctx
        .data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", "project-filter")))
        })
        .expect("project filter geometry");
    let pos = rect.center();
    for events in [
        vec![egui::Event::PointerMoved(pos)],
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }],
    ] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
    }
    paint(&mut app);
    assert!(app.text_input_focused(&ctx));
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
