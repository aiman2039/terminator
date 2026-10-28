use super::*;

struct PaintAttachmentInput<'a> {
    app: &'a mut App,
    ctx: &'a egui::Context,
    session: &'a Session,
    events: Vec<egui::Event>,
}

fn paint_attachment(input: PaintAttachmentInput<'_>) -> Option<egui::Pos2> {
    let PaintAttachmentInput {
        app,
        ctx,
        session,
        events,
    } = input;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            workspace_ui::Viewer {
                app,
                strip: false,
                project: Some("a".into()),
                tab: None,
                window: None,
                render_path: None,
                render_tab: None,
            }
            .terminal_view(ui, session);
        },
    );
    let retry = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == "Retry now" => {
            Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
        }
        _ => None,
    });
    output.textures_delta.clear();
    retry
}

#[test]
fn idle_legacy_or_broken_daemon_is_retired_but_live_sessions_are_preserved() {
    let mut state = State {
        daemon_version: Some(env!("CARGO_PKG_VERSION").into()),
        capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
        ..State::default()
    };
    for health in [None, Some(false)] {
        state.attachment_helper_available = health;
        assert!(can_retire_daemon(&state));
        assert!(!can_restart_service(&state));
        state
            .sessions
            .push(session_fixture("live", SessionKind::Shell));
        assert!(!can_retire_daemon(&state));
        assert!(can_restart_service(&state));
        state.sessions.clear();
    }
    state.attachment_helper_available = Some(true);
    // Even a healthy sibling helper must migrate to a pinned copy when idle.
    assert!(can_retire_daemon(&state));
    state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
    assert!(
        can_retire_daemon(&state),
        "An idle legacy service must migrate to generation ownership"
    );
    state.capabilities.push(generations::CAPABILITY.into());
    assert!(!can_retire_daemon(&state));
    state.attachment_helper_available = Some(false);
    state.capabilities.clear();
    assert!(!can_retire_daemon(&state));
    state.capabilities.push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
    state.daemon_version = Some("999.0.0".into());
    assert!(!can_retire_daemon(&state));
}

#[test]
fn legacy_helper_error_opens_recovery_and_survives_unreported_health() {
    let (mut app, ctx, _dir) = fixture();
    app.state.generation = "old-daemon".into();
    app.update_tx
        .send(Update::Error(
            "Attachment helper unavailable: /AppTranslocation/old/terminator-hook".into(),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.installation_problem());
    app.error = None; // Dismissing a general error must not hide recovery.
    app.apply_state(app.state.clone());
    assert!(app.installation_problem());
    app.open_installation_settings();
    assert!(app.settings_open);
    assert_eq!(app.settings_section, SettingsSection::Updates);
    let mut repaired = app.state.clone();
    repaired.generation = "new-daemon".into();
    repaired.attachment_helper_available = Some(true);
    app.apply_state(repaired);
    assert!(!app.installation_problem());
}

#[test]
fn reconnect_clears_transport_errors_but_preserves_failed_operations() {
    let (mut app, ctx, _dir) = fixture();
    for message in [
        "Reconnecting: Session daemon unavailable",
        "Session daemon unavailable: No such file or directory (os error 2)",
    ] {
        app.update_tx.send(Update::Error(message.into())).unwrap();
        drain_updates(&mut app, &ctx);
        assert!(!app.connected);
        app.update_tx
            .send(Update::State(Box::new(app.state.clone())))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert!(app.connected);
        assert!(app.error.is_none());
    }
    for message in [
        "Settings rejected",
        "Could not save workspace before repair",
    ] {
        app.update_tx.send(Update::Error(message.into())).unwrap();
        drain_updates(&mut app, &ctx);
        app.apply_state(app.state.clone());
        assert_eq!(app.error.as_deref(), Some(message));
    }
}

#[test]
fn dismissed_missing_directory_error_does_not_return() {
    let (mut app, ctx, _dir) = fixture();
    let message = "No such file or directory (os error 2)";
    for _ in 0..2 {
        app.update_tx.send(Update::Error(message.into())).unwrap();
        drain_updates(&mut app, &ctx);
        assert_eq!(app.error.as_deref(), Some(message));
    }
    app.dismiss_status_error();
    assert!(app.error.is_none());
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(
        app.error.is_none(),
        "dismissed missing-path error must stay dismissed"
    );
    app.update_tx
        .send(Update::Error("Settings rejected".into()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.error.as_deref(), Some("Settings rejected"));
}

#[test]
fn dismissed_auth_error_stays_hidden_across_session_navigation() {
    let (mut app, ctx, _dir) = fixture();
    let message = "Cannot read session service authentication file /tmp/old/auth: No such file or directory (os error 2)";
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.error.as_deref(), Some(message));
    app.dismiss_status_error();

    let mut session = session_fixture("shell", SessionKind::Shell);
    session.cwd = "/other".into();
    app.state.sessions.push(session);
    app.selected = Some("a".into());
    app.active_session = Some("shell".into());
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.error.is_none());

    app.update_tx
        .send(Update::Error("Settings rejected".into()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    app.dismiss_status_error();
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.error.is_none());

    let new_owner = "Cannot read session service authentication file /tmp/new/auth: No such file or directory (os error 2)";
    app.update_tx.send(Update::Error(new_owner.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.error.as_deref(), Some(new_owner));
}

#[test]
fn missing_directory_error_stops_after_three_reports() {
    let (mut app, ctx, _dir) = fixture();
    let message = "No such file or directory (os error 2)";
    for _ in 0..retry_budget::MISSING_PATH_RETRY_LIMIT {
        app.update_tx.send(Update::Error(message.into())).unwrap();
        drain_updates(&mut app, &ctx);
        assert_eq!(app.error.as_deref(), Some(message));
        app.error = None;
    }
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.error.is_none());
}

#[test]
fn missing_directory_error_retries_after_the_working_directory_changes() {
    let (mut app, ctx, _dir) = fixture();
    let message = "No such file or directory (os error 2)";
    for _ in 0..retry_budget::MISSING_PATH_RETRY_LIMIT {
        app.update_tx.send(Update::Error(message.into())).unwrap();
        drain_updates(&mut app, &ctx);
        app.error = None;
    }
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.error.is_none());
    let mut session = session_fixture("shell", SessionKind::Shell);
    session.cwd = "/gone".into();
    app.state.sessions.push(session);
    app.selected = Some("a".into());
    app.active_session = Some("shell".into());
    app.update_tx.send(Update::Error(message.into())).unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.error.as_deref(), Some(message));
}

#[test]
fn hover_and_spectrum_failures_do_not_use_the_status_banner() {
    let files = async_service::OperationContext::new(
        "files",
        "hover".into(),
        async_service::Policy::ReplaceableRead,
    );
    assert!(matches!(
        service_failure_update(&files, &async_service::Failure::Overloaded),
        Some(Update::ResolvedTarget(key, None)) if key == "hover"
    ));
    let spectrum = async_service::OperationContext::new(
        "audio-spectrum",
        "player".into(),
        async_service::Policy::ReplaceableRead,
    );
    assert!(service_failure_update(&spectrum, &async_service::Failure::Overloaded).is_none());
    let mutation = async_service::OperationContext::new(
        "daemon",
        "workspace".into(),
        async_service::Policy::OrderedMutation,
    );
    assert!(matches!(
        service_failure_update(&mutation, &async_service::Failure::Overloaded),
        Some(Update::Error(message)) if message == "Services are busy; retry the action"
    ));
}

#[test]
fn expired_gui_request_cannot_change_the_workspace_after_timeout() {
    let (mut app, ctx, _dir) = fixture();
    let (reply, result) = mpsc::sync_channel(1);
    app.update_tx
        .send(Update::UiRequest(
            terminator_core::ui_control::Request::OpenFile {
                project: "b".into(),
                path: "/b/late.txt".into(),
                as_text: true,
            },
            reply,
            Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(result.recv().unwrap().unwrap_err().contains("expired"));
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert!(!app.open_path);
}

#[test]
fn repair_never_queues_shutdown_with_live_or_unsupported_sessions() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.daemon_version = Some(env!("CARGO_PKG_VERSION").into());
    app.state.capabilities = vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()];
    app.state
        .sessions
        .push(session_fixture("unsaved-editor", SessionKind::Editor));
    app.begin_installation_repair();
    assert!(requests.try_recv().is_err());
    app.state.sessions.clear();
    app.state.capabilities.clear();
    app.begin_installation_repair();
    assert!(requests.try_recv().is_err());
    app.state
        .capabilities
        .push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
    app.begin_installation_repair();
    app.begin_installation_repair();
    assert!(app.repair_pending);
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::RepairInstallation(_, _)
    ));
    assert!(requests.try_recv().is_err());
}

#[test]
fn automatic_upgrade_waits_for_idle_and_does_not_retry_failed_generation() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.daemon_version = Some("0.0.1".into());
    app.state.capabilities = vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()];
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.maybe_upgrade_idle_daemon();
    assert!(requests.try_recv().is_err());
    assert!(app.automatic_repair_attempt.is_none());
    app.state.sessions.clear();
    app.exit = exit::Exit::Waiting(Instant::now());
    app.maybe_upgrade_idle_daemon();
    assert!(requests.try_recv().is_err());
    app.exit = exit::Exit::Idle;
    app.maybe_upgrade_idle_daemon();
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::RepairInstallation(_, _)
    ));
    app.repair_pending = false;
    app.maybe_upgrade_idle_daemon();
    assert!(requests.try_recv().is_err());
    app.begin_installation_repair();
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::RepairInstallation(_, _)
    ));
}

#[test]
fn finished_restart_helper_restores_retry() {
    let (mut app, ctx, _dir) = fixture();
    let (tx, rx) = mpsc::channel();
    app.updates = rx;
    app.update_tx = tx.clone();
    app.restart_pending = true;
    tx.send(Update::RestartFinished(String::new())).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.restart_pending);
    assert!(app.error.as_deref().unwrap().contains("restart.log"));
}

#[test]
fn lost_service_is_detected_only_after_state_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    assert!(!app.service_disconnected());
    app.state_loaded = true;
    assert!(app.service_disconnected());
    app.connected = true;
    assert!(!app.service_disconnected());
}

#[test]
fn service_start_queues_one_launch_and_reports() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state_loaded = true;
    app.connected = false;
    app.begin_service_start();
    app.begin_service_start();
    assert!(app.service_start_pending);
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::StartSessionService
    ));
    assert!(requests.try_recv().is_err());
    app.connected = true;
    app.service_start_pending = false;
    app.begin_service_start();
    assert!(!app.service_start_pending);
    assert!(requests.try_recv().is_err());
    app.connected = false;
    let (tx, rx) = mpsc::channel();
    app.updates = rx;
    app.update_tx = tx.clone();
    tx.send(Update::ServiceStarted(Ok(()))).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.service_start_pending);
    assert!(app.info.as_deref().unwrap().contains("Reconnecting"));
    app.service_start_pending = true;
    tx.send(Update::ServiceStarted(Err("gone".into()))).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.service_start_pending);
    assert!(
        app.error
            .as_deref()
            .unwrap()
            .contains("Could not start session service")
    );
}

#[test]
fn unavailable_tabs_are_pruned_without_touching_live_tabs() {
    let (mut app, _, _dir) = fixture();
    app.insert("a", Tab::Terminal("ghost".into()), None);
    // A stale snapshot while disconnected must never report orphans.
    app.connected = false;
    assert!(app.unavailable_tabs().is_empty());
    app.connected = true;
    assert_eq!(app.unavailable_tabs(), vec!["ghost".to_string()]);
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    assert_eq!(app.unavailable_tabs(), vec!["ghost".to_string()]);
    app.close_unavailable_tabs();
    assert!(app.unavailable_tabs().is_empty());
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    assert!(app.info.as_deref().unwrap().contains("Closed 1 tab"));
}

#[test]
fn unavailable_owner_stays_saved_but_disappears_from_running_sidebar() {
    let (mut app, _, directory) = fixture();
    let mut reachable = session_fixture("reachable", SessionKind::Shell);
    reachable.generation = "active".into();
    let mut stranded = session_fixture("stranded", SessionKind::Shell);
    stranded.generation = "old".into();
    app.state.generation = "active".into();
    app.state.sessions = vec![reachable, stranded];
    let active = generation_health(GenerationHealthInput {
        id: "active",
        directory: directory.path(),
        revision: 1,
        status: generations::Status::Active,
    });
    let mut old = generation_health(GenerationHealthInput {
        id: "old",
        directory: directory.path(),
        revision: 1,
        status: generations::Status::Draining,
    });
    old.error = Some("Cannot read authentication file".into());
    app.state.generations = vec![active, old];
    app.insert("a", Tab::Terminal("stranded".into()), None);

    let sidebar = app.sidebar_index();
    assert_eq!(sidebar.live_counts.get("a"), Some(&1));
    assert!(sidebar.sessions.contains_key("stranded"));
    app.remove_tab("stranded");
    assert!(!app.layouts["a"].contains(&Tab::Terminal("stranded".into())));
    assert!(
        app.state
            .sessions
            .iter()
            .any(|session| session.id == "stranded")
    );
    assert_eq!(app.sidebar_index().live_counts.get("a"), Some(&1));

    let mut restored = app.state.clone();
    restored.generations[1].error = None;
    restored.generations[1].revision = 2;
    restored.client_observation = 2;
    app.apply_state(restored);
    assert_eq!(app.sidebar_index().live_counts.get("a"), Some(&2));
}

#[test]
fn unavailable_pane_close_waits_until_the_workspace_is_checked_in() {
    let (mut app, _, _dir) = fixture();
    app.connected = true;
    app.insert("a", Tab::Terminal("ghost".into()), None);
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    let dock = app.layouts.remove("a").unwrap();
    app.remove_tab("ghost");
    app.layouts.insert("a".into(), dock);
    assert!(app.layouts["a"].contains(&Tab::Terminal("ghost".into())));
    let dock = app.layouts.remove("a").unwrap();
    app.queue_unavailable_tab_close("ghost");
    app.layouts.insert("a".into(), dock);
    app.drain_pending_unavailable_close();
    assert!(!app.layouts["a"].contains(&Tab::Terminal("ghost".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    assert!(app.unavailable_tabs().is_empty());
}

#[test]
fn exited_non_resumable_session_closes_its_tab() {
    let (mut app, _ctx, _dir) = fixture();
    app.connected = true;
    app.insert("a", Tab::Terminal("ghost".into()), None);
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.prune_unavailable_tabs();
    assert!(!app.layouts["a"].contains(&Tab::Terminal("ghost".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
}

#[test]
fn restart_is_hidden_for_newer_daemons_and_idle_services() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.daemon_version = Some("999.0.0".into());
    app.state.capabilities = vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()];
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.begin_session_restart();
    assert!(requests.try_recv().is_err());
    app.state.daemon_version = Some("0.0.1".into());
    app.state.sessions.clear();
    app.begin_session_restart();
    assert!(requests.try_recv().is_err());
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.begin_session_restart();
    app.begin_session_restart();
    assert!(app.restart_pending);
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::RestartSessionService(_)
    ));
    assert!(requests.try_recv().is_err());
}

#[test]
fn restart_confirm_cancel_does_not_spawn() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.daemon_version = Some("0.0.1".into());
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.restart_confirm = true;
    app.restart_confirm = false;
    assert!(requests.try_recv().is_err());
    assert!(!app.restart_pending);
}

#[test]
fn restart_job_keeps_the_inventory_at_confirmation() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.generation = "confirmed-owner".into();
    app.state.daemon_version = Some("0.0.1".into());
    app.state.capabilities = vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()];
    app.state
        .sessions
        .push(session_fixture("approved", SessionKind::Shell));
    app.begin_session_restart();
    app.state
        .sessions
        .push(session_fixture("late", SessionKind::Shell));
    let Job::RestartSessionService(inventory) = requests.try_recv().unwrap() else {
        panic!("restart job");
    };
    assert_eq!(inventory.generation, "confirmed-owner");
    assert_eq!(inventory.sessions, ["approved".into()].into());
    assert!(inventory.validate(&app.state).is_err());
}

#[test]
fn restart_cleans_ended_panes_but_history_can_be_reopened() {
    let (mut app, _, _dir) = fixture();
    let mut dock = DockState::new(vec![Tab::Terminal("remaining".into())]);
    dock.main_surface_mut().split_below(
        NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("ended".into())],
    );
    let mut state = app.state.clone();
    state.projects[0].layout = sanitize_layout(serde_json::to_value(&dock).unwrap());
    let mut ended = session_fixture("ended", SessionKind::Shell);
    ended.lifecycle = Lifecycle::Ended;
    state.sessions = vec![session_fixture("remaining", SessionKind::Shell), ended];
    app.layouts.clear();
    app.state_loaded = false;
    app.apply_state(state.clone());
    assert!(app.layouts["a"].main_surface()[NodeIndex::root()].is_leaf());
    app.go_session("ended");
    app.apply_state(state);
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("ended".into()))
            .is_some()
    );
}

#[test]
fn retry_now_bypasses_the_delay_without_creating_a_session() {
    let (mut app, ctx, _dir) = fixture();
    let session = session_fixture("manual-retry", SessionKind::Shell);
    app.state.sessions.push(session.clone());
    let mut retry = retry_budget::AttachmentRetry::default();
    for _ in 0..8 {
        retry.record_failure(std::time::Instant::now());
    }
    app.attach_budget.insert(session.id.clone(), retry);
    app.attach_error
        .insert(session.id.clone(), "Temporary failure".into());
    app.attach_started
        .insert(session.id.clone(), std::time::Instant::now());
    let backend = app.next_backend;
    let pos = paint_attachment(PaintAttachmentInput {
        app: &mut app,
        ctx: &ctx,
        session: &session,
        events: vec![],
    })
    .expect("Retry button");
    paint_attachment(PaintAttachmentInput {
        app: &mut app,
        ctx: &ctx,
        session: &session,
        events: vec![egui::Event::PointerMoved(pos)],
    });
    for pressed in [true, false] {
        paint_attachment(PaintAttachmentInput {
            app: &mut app,
            ctx: &ctx,
            session: &session,
            events: vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }],
        });
    }
    assert!(!app.attach_budget.contains_key(&session.id));
    assert!(!app.attach_error.contains_key(&session.id));
    assert!(!app.attach_started.contains_key(&session.id));
    assert_eq!(app.next_backend, backend);
    assert_eq!(app.state.sessions.len(), 1);
    assert_eq!(app.state.sessions[0].id, session.id);
}

#[test]
fn terminal_retries_after_cooldown_without_a_click_or_new_session() {
    let (mut app, ctx, dir) = fixture();
    let session = session_fixture("retry-original-session", SessionKind::Shell);
    app.state.sessions.push(session.clone());
    // A missing executable makes spawning fail deterministically, without
    // accessing a real daemon or starting a shell.
    app.state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
    app.state.attachment_helper_available = Some(true);
    app.state.attachment_helper_executable = Some(dir.path().join("missing-helper"));
    let mut retry = retry_budget::AttachmentRetry::default();
    retry.record_failure(std::time::Instant::now());
    app.attach_budget.insert(session.id.clone(), retry);
    let before = app.next_backend;
    let paint = |app: &mut App| {
        paint_attachment(PaintAttachmentInput {
            app,
            ctx: &ctx,
            session: &session,
            events: vec![],
        });
    };
    paint(&mut app);
    assert_eq!(app.next_backend, before, "No spawn during cooldown");
    let mut expired = retry_budget::AttachmentRetry::default();
    for _ in 0..8 {
        expired.record_failure(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_mins(1))
                .unwrap(),
        );
    }
    app.attach_budget.insert(session.id.clone(), expired);
    paint(&mut app);
    assert_eq!(app.next_backend, before + 1, "Retry without a user click");
    assert!(app.attach_error[&session.id].starts_with("Cannot attach terminal:"));
    paint(&mut app);
    assert_eq!(
        app.next_backend,
        before + 1,
        "Failed spawn starts a new cooldown"
    );
    assert_eq!(app.state.sessions.len(), 1);
    assert_eq!(app.state.sessions[0].id, session.id);
    assert!(app.state.sessions[0].lifecycle.live());
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
        generation_health(GenerationHealthInput {
            id: "active",
            directory: directory.path(),
            revision: 10,
            status: generations::Status::Active,
        }),
        generation_health(GenerationHealthInput {
            id: "retired",
            directory: directory.path(),
            revision: 50,
            status: generations::Status::Retired,
        }),
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
    drain_updates(&mut app, &ctx);
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("shell".into()))
            .is_none()
    );
}

#[test]
fn unavailable_owner_keeps_last_records_but_updates_health() {
    let (mut app, ctx, directory) = fixture();
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
    app.apply_state(state.clone());
    assert!(
        !app.connected,
        "A saved snapshot is not a live service connection"
    );
    assert!(
        app.state
            .sessions
            .iter()
            .any(|session| session.id == "last-observed")
    );
    assert!(app.state.generations[0].error.is_some());
    let session = app
        .state
        .sessions
        .iter()
        .find(|s| s.id == "last-observed")
        .unwrap()
        .clone();
    let backend = app.next_backend;
    paint_attachment(PaintAttachmentInput {
        app: &mut app,
        ctx: &ctx,
        session: &session,
        events: vec![],
    });
    assert_eq!(
        app.next_backend, backend,
        "An unavailable owner must not receive attachment attempts"
    );
    assert!(!app.backends.contains_key(&session.id));
    let mut budget = retry_budget::AttachmentRetry::default();
    for _ in 0..retry_budget::MISSING_PATH_RETRY_LIMIT {
        budget.record_failure(std::time::Instant::now());
    }
    app.attach_budget.insert(session.id.clone(), budget);
    app.attach_error
        .insert(session.id.clone(), "attachment failed".into());
    state.sessions = app.state.sessions.clone();
    state.revision = 21;
    state.generations[0].revision = 21;
    state.generations[0].error = None;
    app.apply_state(state);
    assert!(app.connected);
    assert!(!app.attach_budget.contains_key("last-observed"));
    assert!(!app.attach_error.contains_key("last-observed"));
    assert!(app.state.generations[0].error.is_none());
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
