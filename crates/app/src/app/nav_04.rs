use eframe::egui::{self};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{self},
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
use crate::preferences::ProjectSort;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_helper_error_opens_recovery_and_survives_unreported_health() {
        let (mut app, ctx, _dir) = fixture();
        app.state.generation = "old-daemon".into();
        app.update_tx
            .send(Update::Error(
                "Attachment helper unavailable: /AppTranslocation/old/terminator-hook".into(),
            ))
            .unwrap();
        app.process_updates(&ctx);
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
            app.process_updates(&ctx);
            assert!(!app.connected);
            app.update_tx
                .send(Update::State(Box::new(app.state.clone())))
                .unwrap();
            app.process_updates(&ctx);
            assert!(app.connected);
            assert!(app.error.is_none());
        }
        for message in [
            "Settings rejected",
            "Could not save workspace before repair",
        ] {
            app.update_tx.send(Update::Error(message.into())).unwrap();
            app.process_updates(&ctx);
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
            app.process_updates(&ctx);
            assert_eq!(app.error.as_deref(), Some(message));
        }
        app.dismiss_status_error();
        assert!(app.error.is_none());
        app.update_tx.send(Update::Error(message.into())).unwrap();
        app.process_updates(&ctx);
        assert!(
            app.error.is_none(),
            "dismissed missing-path error must stay dismissed"
        );
        app.update_tx
            .send(Update::Error("Settings rejected".into()))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.error.as_deref(), Some("Settings rejected"));
    }

    #[test]
    fn missing_directory_error_stops_after_three_reports() {
        let (mut app, ctx, _dir) = fixture();
        let message = "No such file or directory (os error 2)";
        for _ in 0..retry_budget::MISSING_PATH_RETRY_LIMIT {
            app.update_tx.send(Update::Error(message.into())).unwrap();
            app.process_updates(&ctx);
            assert_eq!(app.error.as_deref(), Some(message));
            app.error = None;
        }
        app.update_tx.send(Update::Error(message.into())).unwrap();
        app.process_updates(&ctx);
        assert!(app.error.is_none());
    }

    #[test]
    fn missing_directory_error_retries_after_the_working_directory_changes() {
        let (mut app, ctx, _dir) = fixture();
        let message = "No such file or directory (os error 2)";
        for _ in 0..retry_budget::MISSING_PATH_RETRY_LIMIT {
            app.update_tx.send(Update::Error(message.into())).unwrap();
            app.process_updates(&ctx);
            app.error = None;
        }
        app.update_tx.send(Update::Error(message.into())).unwrap();
        app.process_updates(&ctx);
        assert!(app.error.is_none());
        let mut session = session_fixture("shell", SessionKind::Shell);
        session.cwd = "/gone".into();
        app.state.sessions.push(session);
        app.selected = Some("a".into());
        app.active_session = Some("shell".into());
        app.update_tx.send(Update::Error(message.into())).unwrap();
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
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
        app.restart_pending = true;
        tx.send(Update::RestartFinished(String::new())).unwrap();
        app.process_updates(&ctx);
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
        tx.send(Update::ServiceStarted(Ok(()))).unwrap();
        app.process_updates(&ctx);
        assert!(!app.service_start_pending);
        assert!(app.info.as_deref().unwrap().contains("Reconnecting"));
        app.service_start_pending = true;
        tx.send(Update::ServiceStarted(Err("gone".into()))).unwrap();
        app.process_updates(&ctx);
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
        app.state.notifications.push(notice_fixture(
            "notice",
            "shell",
            AgentState::WaitingInput,
            1,
        ));
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
    fn explorer_prepared_rows_reuse_listings_and_refresh_expansion_and_content() {
        let (mut app, _, _dir) = fixture();
        let root = PathBuf::from("/a");
        let folder = root.join("folder");
        app.dirs.insert(
            root.clone(),
            vec![
                terminator_git::Entry {
                    path: folder.clone(),
                    directory: true,
                    ignored: false,
                },
                terminator_git::Entry {
                    path: root.join("ignored.rs"),
                    directory: false,
                    ignored: true,
                },
            ],
        );
        app.dirs.insert(
            folder.clone(),
            vec![terminator_git::Entry {
                path: folder.join("file.rs"),
                directory: false,
                ignored: false,
            }],
        );
        let rows = app.explorer_rows(&root);
        assert_eq!(rows.len(), 1);
        assert!(std::sync::Arc::ptr_eq(&rows, &app.explorer_rows(&root)));
        app.expanded_dirs.insert(folder.clone());
        let expanded = app.explorer_rows(&root);
        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[1].depth, 1);
        app.visible_dirs.clear();
        assert!(std::sync::Arc::ptr_eq(&expanded, &app.explorer_rows(&root)));
        assert_eq!(app.visible_dirs, [root.clone(), folder.clone()]);
        app.preferences.show_ignored = true;
        assert_eq!(app.explorer_rows(&root).len(), 3);
        app.explorer_query = "missing".into();
        assert_eq!(app.explorer_rows(&root).len(), 1); // folders remain navigable
        app.explorer_query.clear();
        app.dirs.get_mut(&folder).unwrap()[0].path = folder.join("changed.rs");
        app.sidebar_cache.get_mut().clear_files();
        assert_eq!(app.explorer_rows(&root)[1].label, "changed.rs");
    }

    #[test]
    fn search_results_preserve_headers_and_row_heights_when_clipped() {
        let (mut app, ctx, _dir) = fixture();
        app.explorer_search = (0..50)
            .map(|i| search::Hit {
                path: PathBuf::from(format!("/a/{}.rs", i / 5)),
                line: i + 1,
                text: format!("hit {i}"),
            })
            .collect();
        let mut heights = Vec::new();
        for clipped in [false, true] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(300.0);
                ui.set_clip_rect(if clipped {
                    egui::Rect::NOTHING
                } else {
                    egui::Rect::EVERYTHING
                });
                let start = ui.next_widget_position().y;
                app.explorer_results(ui, Path::new("/a"));
                heights.push(ui.next_widget_position().y - start);
            });
            output.textures_delta.clear();
        }
        assert_eq!(heights[0], heights[1]);
        assert!(heights[0] > 50.0 * 20.0); // includes all ten file headers
    }

    #[test]
    fn accepted_filesystem_refresh_replaces_prepared_git_and_explorer_data() {
        let (mut app, ctx, _dir) = fixture();
        let root = PathBuf::from("/a");
        let context = services::ContextData {
            cwd: root.clone(),
            root: Some(root.clone()),
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![],
            decorations: HashMap::default(),
            stats: HashMap::default(),
            error: None,
        };
        app.dirs.insert(root.clone(), vec![]);
        let rows = app.explorer_rows(&root);
        app.sidebar_cache.get_mut().git =
            Some(std::sync::Arc::new(sidebar_ui::PreparedGit::new(&context)));
        app.refresh_generation = 7;
        app.refresh_request = Some(refresh::Request {
            cwd: root.clone(),
            generation: 7,
            directories: vec![root.clone()],
        });
        app.update_tx
            .send(Update::Refresh(6, context.clone(), vec![], false))
            .unwrap();
        app.process_updates(&ctx);
        assert!(std::sync::Arc::ptr_eq(&rows, &app.explorer_rows(&root)));
        assert!(app.sidebar_cache.borrow().git.is_some());
        app.update_tx
            .send(Update::Refresh(
                7,
                context,
                vec![(
                    root.clone(),
                    Ok(vec![terminator_git::Entry {
                        path: root.join("new.rs"),
                        directory: false,
                        ignored: false,
                    }]),
                )],
                false,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.sidebar_cache.borrow().git.is_none());
        let updated = app.explorer_rows(&root);
        assert!(!std::sync::Arc::ptr_eq(&rows, &updated));
        assert_eq!(updated[0].label, "new.rs");
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
        drain_until_project_activity(&mut app, &ctx, "a", 2);
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
        drain_until_project_activity(&mut app, &ctx, "a", 2);
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
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
        assert!(app.preferences.hidden_projects.contains("a"));
        assert_eq!(app.selected.as_deref(), Some("b"));
    }
}
