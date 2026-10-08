use super::super::appearance;
use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use std::sync::mpsc::{self};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "test-support")]
    fn attention_row_glyphs_share_one_center_line() {
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
                    egui::vec2(320.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.agents_view(ui);
            },
        );
        output.textures_delta.clear();
        let action = agent_target(&ctx, "agent-go:live-shell").unwrap();
        let status = agent_target(&ctx, "agent-status:live-shell").unwrap();
        assert!(
            (status.height() - action.height()).abs() < 1.0,
            "status {} should match action height {}",
            status.height(),
            action.height()
        );
        assert!(
            (status.center().y - action.center().y).abs() < 1.0,
            "status center {} should match action center {}",
            status.center().y,
            action.center().y
        );
        fn label_centers(out: &mut Vec<f32>, shape: &egui::Shape, needle: &str) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        label_centers(out, shape, needle);
                    }
                }
                egui::Shape::Text(text) if text.galley.text() == needle => {
                    out.push(text.visual_bounding_rect().center().y);
                }
                _ => {}
            }
        }
        let mut centers = Vec::new();
        for clipped in &output.shapes {
            label_centers(&mut centers, &clipped.shape, "live-shell");
        }
        assert_eq!(
            centers.len(),
            1,
            "expected one painted row label, got {centers:?}"
        );
        assert!(
            (centers[0] - action.center().y).abs() < 2.0,
            "label center {} should match action center {}",
            centers[0],
            action.center().y
        );
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

    #[test]
    fn waiting_badge_matches_pending_notices_not_live_agents() {
        fn update_notice(app: &mut App, edit: impl FnOnce(&mut Notification)) {
            let mut state = app.state.clone();
            edit(&mut state.notifications[0]);
            app.apply_state(state);
        }
        let (mut app, _, _dir) = fixture();
        app.selected = Some("a".into());
        app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
        app.state.agents = vec![Agent {
            invocation_id: "agent".into(),
            session_id: "s".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            sequence: Some(1),
            updated: 0,
            resume: None,
            process: None,
        }];
        assert_eq!(app.waiting_notice_count(), 0);
        app.state.notifications = vec![Notification {
            id: "n".into(),
            session_id: "s".into(),
            invocation_id: "agent".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: "Need input".into(),
            details: String::new(),
            created: 1,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        }];
        assert_eq!(app.waiting_notice_count(), 1);
        update_notice(&mut app, |notice| {
            notice.state = AgentState::WaitingPermission;
        });
        assert_eq!(app.waiting_notice_count(), 1);
        update_notice(&mut app, |notice| notice.dismissed = true);
        assert_eq!(app.waiting_notice_count(), 0);
        update_notice(&mut app, |notice| notice.dismissed = false);
        update_notice(&mut app, |notice| notice.snoozed_until = now() + 600);
        assert_eq!(app.waiting_notice_count(), 0);
        update_notice(&mut app, |notice| notice.snoozed_until = 0);
        // The inbox spans all projects: switching projects keeps the badge.
        app.selected = Some("b".into());
        assert_eq!(app.waiting_notice_count(), 1);
        update_notice(&mut app, |notice| notice.resolved = true);
        assert_eq!(app.waiting_notice_count(), 0);
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn resolved_waiting_notice_is_not_listed() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("done", SessionKind::Shell),
            session_fixture("resolved", SessionKind::Shell),
        ];
        let mut resolved =
            notice_fixture("old-wait", "resolved", AgentState::WaitingPermission, now());
        resolved.resolved = true;
        app.state.notifications = vec![
            resolved,
            notice_fixture("done", "done", AgentState::Completed, 1),
        ];
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "agent-row:done").is_some());
        assert!(agent_target(&ctx, "agent-row:resolved").is_none());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_lists_terminal_notices() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.terminal_notices = vec![TerminalNotice {
            id: "tn".into(),
            session_id: "live-shell".into(),
            title: "Terminal".into(),
            body: "bell".into(),
            created: 1,
            dismissed: false,
        }];
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "terminal-row:live-shell").is_some());
        assert!(agent_target(&ctx, "terminal-go:live-shell").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_lists_pending_notices_not_stopped_agents() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("stopped-shell", SessionKind::Shell),
            session_fixture("live-shell", SessionKind::Shell),
        ];
        app.state.agents = vec![Agent {
            invocation_id: "stopped".into(),
            session_id: "stopped-shell".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::Stopped,
            sequence: None,
            updated: 1,
            resume: None,
            process: None,
        }];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "agent-row:stopped-shell").is_none());
        assert!(agent_target(&ctx, "agent-row:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-go:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-snooze:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-dismiss:live-shell").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_puts_waiting_above_completed() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("done-shell", SessionKind::Shell),
            session_fixture("live-shell", SessionKind::Shell),
        ];
        app.state.notifications = vec![
            notice_fixture("done", "done-shell", AgentState::Completed, now()),
            notice_fixture("wait", "live-shell", AgentState::WaitingPermission, 1),
        ];
        render_agents(&mut app, &ctx, vec![]);
        let waiting = agent_target(&ctx, "agent-row:live-shell").unwrap();
        let completed = agent_target(&ctx, "agent-row:done-shell").unwrap();
        assert!(waiting.top() < completed.top());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_go_focuses_session_and_dismiss_hides_card() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.apply_notice_action("wait".into(), AttentionAction::Go);
        assert_eq!(app.active_session.as_deref(), Some("live-shell"));
        app.apply_notice_action("wait".into(), AttentionAction::Dismiss);
        assert!(
            app.state
                .notifications
                .iter()
                .all(|notice| notice.dismissed)
        );
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "agent-row:live-shell").is_none());
        let mut saw_dismiss = false;
        while let Ok(job) = received.try_recv() {
            if let Job::Control(request, _) = job
                && matches!(
                    *request,
                    Request::Notice {
                        ref action,
                        ..
                    } if action == "dismiss"
                )
            {
                saw_dismiss = true;
            }
        }
        assert!(saw_dismiss);
    }

    #[test]
    fn status_menu_lists_pending_notices_in_inbox_order() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("done-shell", SessionKind::Shell),
            session_fixture("live-shell", SessionKind::Shell),
        ];
        app.state.notifications = vec![
            notice_fixture("done", "done-shell", AgentState::Completed, now()),
            notice_fixture("wait", "live-shell", AgentState::WaitingPermission, 1),
        ];
        let items = app.status_menu_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "wait");
        assert!(items[0].title.contains("live-shell"));
        assert!(items[0].title.contains("Needs permission"));
        assert_eq!(items[1].id, "done");
    }

    #[test]
    fn status_menu_skips_settled_notices_and_caps_single_line_titles() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        let mut long = notice_fixture("long", "live-shell", AgentState::WaitingInput, now());
        long.summary = "line one\nline two ".to_string() + &"word ".repeat(40);
        let mut dismissed =
            notice_fixture("dismissed", "live-shell", AgentState::WaitingInput, now());
        dismissed.dismissed = true;
        let mut resolved =
            notice_fixture("resolved", "live-shell", AgentState::WaitingInput, now());
        resolved.resolved = true;
        let mut snoozed = notice_fixture("snoozed", "live-shell", AgentState::WaitingInput, now());
        snoozed.snoozed_until = now() + 600;
        let mut overflow: Vec<_> = (0..13)
            .map(|index| {
                notice_fixture(
                    &format!("extra-{index}"),
                    "live-shell",
                    AgentState::Completed,
                    now(),
                )
            })
            .collect();
        let mut notices = vec![long, dismissed, resolved, snoozed];
        notices.append(&mut overflow);
        app.state.notifications = notices;
        let items = app.status_menu_items();
        assert_eq!(items.len(), 12);
        assert_eq!(items[0].id, "long");
        assert!(!items[0].title.contains('\n'));
        assert!(items[0].title.chars().count() <= 90);
        assert!(items.iter().all(|item| item.id != "dismissed"));
        assert!(items.iter().all(|item| item.id != "resolved"));
        assert!(items.iter().all(|item| item.id != "snoozed"));
    }

    #[test]
    fn status_menu_pick_focuses_the_waiting_agent() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        let (jobs, _received) = mpsc::channel();
        app.jobs = jobs.into();
        app.preferences.tool = SidebarTool::Explorer;
        app.preferences.visible = false;
        app.detail = Some("wait".into());
        app.focus_status_notice(&ctx, "wait");
        assert_eq!(app.preferences.tool, SidebarTool::Agents);
        assert!(app.preferences.visible);
        assert_eq!(app.active_session.as_deref(), Some("live-shell"));
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert_eq!(app.detail, None);
    }

    #[test]
    fn status_menu_pick_with_stale_id_just_opens_the_inbox() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.tool = SidebarTool::Explorer;
        app.preferences.visible = false;
        app.focus_status_notice(&ctx, "gone");
        assert_eq!(app.preferences.tool, SidebarTool::Agents);
        assert!(app.preferences.visible);
        assert_eq!(app.active_session, None);
        assert_eq!(app.detail, None);
    }

    #[test]
    fn attention_counts_share_waiting_and_unread_with_both_bells() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("one", SessionKind::Shell),
            session_fixture("two", SessionKind::Shell),
        ];
        let mut waiting = notice_fixture("wait", "one", AgentState::WaitingPermission, now());
        waiting.read = true;
        app.state.notifications = vec![
            waiting,
            notice_fixture("done", "two", AgentState::Completed, now()),
        ];
        assert_eq!(app.attention_counts(), (1, 1));
    }

    #[test]
    fn next_attention_cycles_waiting_then_failed_oldest_first() {
        let (mut app, _ctx, _dir) = fixture();
        let mut failed = session_fixture("failed", SessionKind::Shell);
        failed.project_id = "b".into();
        let mut ended = session_fixture("ended", SessionKind::Shell);
        ended.lifecycle = Lifecycle::Ended;
        app.state.sessions = vec![
            session_fixture("early", SessionKind::Shell),
            session_fixture("late", SessionKind::Shell),
            failed,
            ended,
        ];
        let mut notices = vec![
            notice_fixture("w-early", "early", AgentState::WaitingPermission, 10),
            notice_fixture("w-late", "late", AgentState::WaitingInput, 20),
            // Older than every waiting notice, still sorted after the group.
            notice_fixture("f-failed", "failed", AgentState::Failed, 5),
            // A session with both appears once, in the waiting group.
            notice_fixture("f-early-too", "early", AgentState::Failed, 1),
        ];
        let mut dismissed = notice_fixture("dismissed", "late", AgentState::WaitingInput, 0);
        dismissed.dismissed = true;
        let mut snoozed = notice_fixture("snoozed", "late", AgentState::WaitingInput, 0);
        snoozed.snoozed_until = now() + 600;
        let mut resolved = notice_fixture("resolved", "late", AgentState::WaitingInput, 0);
        resolved.resolved = true;
        let gone = notice_fixture("gone", "ended", AgentState::WaitingInput, 0);
        notices.extend([dismissed, snoozed, resolved, gone]);
        app.state.notifications = notices;
        assert_eq!(app.attention_order(), vec!["early", "late", "failed"]);
        app.active_session = Some("late".into());
        app.next_attention();
        assert_eq!(app.active_session.as_deref(), Some("failed"));
        app.active_session = Some("outside".into());

        app.next_attention();
        assert_eq!(app.active_session.as_deref(), Some("early"));
        assert_eq!(app.selected.as_deref(), Some("a"));
        app.next_attention();
        assert_eq!(app.active_session.as_deref(), Some("late"));
        // Cross-project navigation reuses session navigation.
        app.next_attention();
        assert_eq!(app.active_session.as_deref(), Some("failed"));
        assert_eq!(app.selected.as_deref(), Some("b"));
        app.next_attention();
        assert_eq!(app.active_session.as_deref(), Some("early"));
    }

    #[test]
    fn next_attention_reports_an_empty_queue_without_moving() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
        app.next_attention();
        assert_eq!(app.active_session, None);
        assert_eq!(app.info.as_deref(), Some("No agents need attention"));
    }

    #[test]
    fn reading_newest_unread_row_keeps_its_position() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
        app.state.notifications = vec![
            notice_fixture("old", "s", AgentState::WaitingInput, 1),
            notice_fixture("new", "s", AgentState::WaitingInput, 2),
        ];
        app.apply_notice_action("new".into(), AttentionAction::Read);
        assert_eq!(
            app.unread_notices()
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "old"]
        );
    }

    #[test]
    fn unread_selection_survives_marking_read_without_touching_lifecycle() {
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
        app.state.agents = vec![Agent {
            invocation_id: "agent".into(),
            session_id: "s".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            sequence: None,
            updated: 0,
            resume: None,
            process: None,
        }];
        app.state.notifications = vec![
            notice_fixture("n1", "s", AgentState::WaitingInput, 1),
            notice_fixture("n2", "s", AgentState::WaitingInput, 2),
        ];
        assert_eq!(app.unread_notices().len(), 2);
        app.apply_notice_action("n1".into(), AttentionAction::Read);
        let first = app
            .state
            .notifications
            .iter()
            .find(|n| n.id == "n1")
            .unwrap();
        assert!(first.read);
        assert!(!first.resolved && !first.dismissed);
        assert_eq!(app.state.agents[0].state, AgentState::WaitingInput);
        // Retained until selection changes, even though it is now read.
        assert!(app.unread_notices().iter().any(|n| n.id == "n1"));
        app.apply_notice_action("n2".into(), AttentionAction::Read);
        assert_eq!(app.unread_notices().first().unwrap().id, "n2");
        assert!(!app.unread_notices().iter().any(|n| n.id == "n1"));
        assert!(app.unread_notices().iter().any(|n| n.id == "n2"));
        // Switching projects keeps retained rows: the inbox spans all projects.
        app.selected = Some("b".into());
        assert!(app.unread_notices().iter().any(|n| n.id == "n2"));
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn all_live_groups_verified_agents_with_an_unverified_section() {
        use terminator_core::agents::{DetectedAgent, PresenceOutcome, ProcessIdentity};
        let (mut app, ctx, _dir) = fixture();
        app.preferences.agents_tab = AgentsTab::AllLive;
        let mut live_b = session_fixture("live-b", SessionKind::Shell);
        live_b.project_id = "b".into();
        app.state.sessions = vec![
            session_fixture("live-a", SessionKind::Shell),
            live_b,
            session_fixture("hook-only", SessionKind::Shell),
        ];
        app.state
            .capabilities
            .push(AGENT_PRESENCE_CAPABILITY.into());
        for (sid, kind) in [("live-a", "codex"), ("live-b", "claude")] {
            app.state
                .presence
                .push(terminator_core::agents::TerminalPresence {
                    session_id: sid.into(),
                    generation: "fixture".into(),
                    agents: vec![DetectedAgent {
                        kind: kind.into(),
                        process: ProcessIdentity {
                            pid: 200,
                            start_time: 500,
                        },
                        foreground: true,
                    }],
                    outcome: PresenceOutcome::Verified,
                    observed_at: now(),
                });
        }
        app.state.agents = vec![Agent {
            invocation_id: "hook".into(),
            session_id: "hook-only".into(),
            kind: "grok".into(),
            provider_session_id: None,
            state: AgentState::Running,
            sequence: None,
            updated: now(),
            resume: None,
            process: None,
        }];
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "live-project:a").is_some());
        assert!(agent_target(&ctx, "live-project:b").is_some());
        assert!(agent_target(&ctx, "live-row:live-a").is_some());
        assert!(agent_target(&ctx, "live-row:live-b").is_some());
        assert!(agent_target(&ctx, "live-unverified").is_some());
        assert!(agent_target(&ctx, "unverified-row:hook-only").is_some());
        // Text search narrows across projects, kinds, and labels. Fresh
        // contexts per phase: fixture targets persist across renders.
        let search_ctx = egui::Context::default();
        app.preferences.agents_search = "claude".into();
        render_agents(&mut app, &search_ctx, vec![]);
        assert!(agent_target(&search_ctx, "live-row:live-a").is_none());
        assert!(agent_target(&search_ctx, "live-row:live-b").is_some());
        assert!(agent_target(&search_ctx, "unverified-row:hook-only").is_none());
        // The agent filter keeps only the selected kind.
        let filter_ctx = egui::Context::default();
        app.preferences.agents_search.clear();
        app.preferences.agents_filter = "codex".into();
        render_agents(&mut app, &filter_ctx, vec![]);
        assert!(agent_target(&filter_ctx, "live-row:live-a").is_some());
        assert!(agent_target(&filter_ctx, "live-row:live-b").is_none());
        assert!(agent_target(&filter_ctx, "unverified-row:hook-only").is_none());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn workspace_tab_shows_focused_identity_with_aggregate_attention() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("one", SessionKind::Shell),
            session_fixture("two", SessionKind::Shell),
        ];
        app.state.notifications = vec![
            notice_fixture("wait", "two", AgentState::WaitingPermission, 1),
            notice_fixture("broke", "one", AgentState::Failed, 2),
        ];
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(DockState::new(vec![
                Tab::Terminal("one".into()),
                Tab::Terminal("two".into()),
            ])),
        );
        app.selected = Some("a".into());
        combined_frame(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "workspace-tab:one").is_some());
        assert!(agent_target(&ctx, "workspace-tab-attention:one").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn dismiss_click_does_not_focus_the_agent() {
        let (mut app, ctx, _dir, received) = waiting_inbox();
        click_agent_target(&mut app, &ctx, "agent-dismiss:live-shell");
        assert!(app.state.notifications[0].dismissed);
        assert!(app.active_session.is_none());
        let actions = control_actions(&received);
        assert!(actions.iter().any(|action| action == "dismiss"));
        assert!(!actions.iter().any(|action| action == "focus"));
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn snooze_click_does_not_focus_the_agent() {
        let (mut app, ctx, _dir, received) = waiting_inbox();
        click_agent_target(&mut app, &ctx, "agent-snooze:live-shell");
        assert!(app.state.notifications[0].snoozed_until > now());
        assert!(!app.state.notifications[0].dismissed);
        assert!(app.active_session.is_none());
        let actions = control_actions(&received);
        assert!(actions.iter().any(|action| action == "snooze"));
        assert!(!actions.iter().any(|action| action == "focus"));
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn go_click_focuses_the_agent() {
        let (mut app, ctx, _dir, received) = waiting_inbox();
        click_agent_target(&mut app, &ctx, "agent-go:live-shell");
        assert_eq!(app.active_session.as_deref(), Some("live-shell"));
        assert!(
            control_actions(&received)
                .iter()
                .any(|action| action == "focus")
        );
    }
}
