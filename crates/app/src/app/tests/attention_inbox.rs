use super::*;

#[test]
fn attention_migration_retries_without_ack_and_preserves_later_choices() {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.preferences_writable = true;
    app.migrate_attention();
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::MigrateAttention
    ));
    assert!(app.attention_requested.is_some());
    assert!(!app.preferences.attention_migrated);
    let unacknowledged = Instant::now().checked_sub(Duration::from_secs(6)).unwrap();
    app.attention_requested = Some(unacknowledged);
    app.migrate_attention();
    assert_eq!(app.attention_requested, Some(unacknowledged));
    assert!(
        requests.try_recv().is_err(),
        "Pending migration must not be duplicated"
    );
    app.update_tx
        .send(Update::AttentionMigrated(Err("rejected".into())))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.preferences.attention_migrated);
    assert!(!app.attention_pending);
    assert!(app.error.as_deref().unwrap().contains("rejected"));
    app.attention_requested = Some(
        Instant::now()
            .checked_sub(Duration::from_secs(6))
            .unwrap_or_else(Instant::now),
    );
    let previous_request = app.attention_requested.unwrap();
    app.migrate_attention();
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::MigrateAttention
    ));
    assert!(app.attention_pending);
    assert!(app.attention_requested.unwrap() > previous_request);
    app.update_tx
        .send(Update::AttentionMigrated(Ok(())))
        .unwrap();
    drain_updates(&mut app, &ctx);
    app.preferences.save(dir.path()).unwrap();
    assert!(UiPreferences::load(dir.path()).unwrap().attention_migrated);
    app.state.settings.notifications_side = false;
    app.attention_requested = None;
    app.migrate_attention();
    assert!(app.attention_requested.is_none());
    assert!(!app.state.settings.notifications_side);
    assert!(
        requests.try_recv().is_err(),
        "Acknowledged migration must preserve later choices"
    );
}

#[test]
fn failed_migration_does_not_set_marker() {
    let (mut app, ctx, _dir) = fixture();
    app.update_tx
        .send(Update::Error("Settings rejected".into()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.preferences.typography_migrated);
    app.update_tx.send(Update::TypographyMigrated).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.preferences.typography_migrated);
}

#[test]
fn bell_opens_agents_over_an_open_left_tool() {
    let (mut app, _ctx, _dir) = fixture();
    app.preferences.left_visible = true;
    app.preferences.left_tool = Some(SidebarTool::Git);
    // A stale flag hidden behind the tool must not make the bell close.
    app.preferences.left_agents = true;
    assert!(!app.left_agents_shown());
    app.toggle_left_agents();
    assert!(app.left_agents_shown());
    assert_eq!(app.preferences.left_tool, None);
    app.toggle_left_agents();
    assert!(!app.left_agents_shown());
    assert!(!app.preferences.left_agents);
}

#[test]
fn opening_the_inbox_replaces_projects_on_the_right() {
    let (mut app, _ctx, _dir) = fixture();
    app.preferences.right_projects = true;
    app.preferences.visible = false;
    app.open_agents_inbox();
    assert!(!app.preferences.right_projects);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Agents);
}

#[test]
fn bell_shows_a_hidden_left_sidebar() {
    let (mut app, _ctx, _dir) = fixture();
    app.preferences.left_visible = false;
    app.preferences.left_agents = false;
    app.toggle_left_agents();
    assert!(app.preferences.left_visible);
    assert!(app.left_agents_shown());
}

#[test]
fn hidden_agent_terminal_keeps_owner_and_directory_context() {
    let (mut app, _, _dir) = fixture();
    app.state.sessions.push(Session {
        remote: None,
        review: false,
        id: "hidden".into(),
        project_id: "a".into(),
        label: "Shell".into(),
        cwd: "/b".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Running,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "same".into(),
        pid: Some(123),
        truncated: false,
        cwd_confirmed: true,
    });
    app.select_project("b".into());
    app.go_session("hidden");
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("hidden".into()))
            .is_some()
    );
    assert_eq!(app.state.sessions[0].pid, Some(123));
    app.terminal_context.insert("a".into(), "hidden".into());
    app.active_session = None; // diff/editor focus retains the preceding shell.
    assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
}

#[test]
#[cfg(feature = "test-support")]
fn inline_attention_preserves_input_but_missing_session_details_remain_modal() {
    let (mut app, _, _dir) = fixture();
    app.preferences.left_agents = true;
    app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
    app.detail = Some("wait".into());
    assert!(!app.notice_detail_modal_open());
    app.preferences.left_agents = false;
    app.preferences.visible = false;
    assert!(app.notice_detail_modal_open());
    app.preferences.left_agents = true;
    app.selected = Some("different-project".into());
    // The inbox spans all projects, so switching projects keeps inline detail.
    assert!(!app.notice_detail_modal_open());
    app.selected = None;
    // Only a missing session drops the notice out of scope.
    let sessions = std::mem::take(&mut app.state.sessions);
    assert!(app.notice_detail_modal_open());
    app.state.sessions = sessions;
    app.state.notifications[0].snoozed_until = now() + 600;
    assert!(app.notice_detail_modal_open());
    app.state.notifications[0].snoozed_until = 0;
    app.state.notifications[0].resolved = true;
    assert!(app.notice_detail_modal_open());
    app.state.notifications[0].resolved = false;
    app.state.notifications[0].dismissed = true;
    assert!(app.notice_detail_modal_open());
}

#[test]
#[cfg(feature = "test-support")]
fn attention_bell_lives_on_the_left_sidebar_only() {
    let (mut app, ctx, _dir) = fixture();
    app.state.settings.notifications_side = true;
    app.preferences.visible = true;
    app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
    let target = |name: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
    };
    // The right sidebar never paints the bell, even with waiting notices.
    for tool in [
        SidebarTool::History,
        SidebarTool::Git,
        SidebarTool::Explorer,
        SidebarTool::Agents,
        SidebarTool::Info,
    ] {
        app.preferences.tool = tool;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if app.preferences.visible {
                app.sidebar(ui);
            }
        });
        output.textures_delta.clear();
        assert!(target("attention-bell").is_none());
    }
    assert!(target("agent-go:live-shell").is_some());
    // The project header keeps the bell, beside the hide control.
    app.project_width = 360.0;
    paint_header(&mut app, &ctx, &[]);
    assert!(target("left-agent-bar").is_some());
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
        app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
            id: "wait",
            session: "live-shell",
            state: AgentState::WaitingPermission,
            created: now(),
        })];
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
                    let rect = agent_target(&ctx, &format!("agent-{action}:live-shell")).unwrap();
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
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
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
    let mut resolved = notice_fixture(NoticeFixtureInput {
        id: "old-wait",
        session: "resolved",
        state: AgentState::WaitingPermission,
        created: now(),
    });
    resolved.resolved = true;
    app.state.notifications = vec![
        resolved,
        notice_fixture(NoticeFixtureInput {
            id: "done",
            session: "done",
            state: AgentState::Completed,
            created: 1,
        }),
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
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
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
        notice_fixture(NoticeFixtureInput {
            id: "done",
            session: "done-shell",
            state: AgentState::Completed,
            created: now(),
        }),
        notice_fixture(NoticeFixtureInput {
            id: "wait",
            session: "live-shell",
            state: AgentState::WaitingPermission,
            created: 1,
        }),
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
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
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
        notice_fixture(NoticeFixtureInput {
            id: "done",
            session: "done-shell",
            state: AgentState::Completed,
            created: now(),
        }),
        notice_fixture(NoticeFixtureInput {
            id: "wait",
            session: "live-shell",
            state: AgentState::WaitingPermission,
            created: 1,
        }),
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
    let mut long = notice_fixture(NoticeFixtureInput {
        id: "long",
        session: "live-shell",
        state: AgentState::WaitingInput,
        created: now(),
    });
    long.summary = "line one\nline two ".to_string() + &"word ".repeat(40);
    let mut dismissed = notice_fixture(NoticeFixtureInput {
        id: "dismissed",
        session: "live-shell",
        state: AgentState::WaitingInput,
        created: now(),
    });
    dismissed.dismissed = true;
    let mut resolved = notice_fixture(NoticeFixtureInput {
        id: "resolved",
        session: "live-shell",
        state: AgentState::WaitingInput,
        created: now(),
    });
    resolved.resolved = true;
    let mut snoozed = notice_fixture(NoticeFixtureInput {
        id: "snoozed",
        session: "live-shell",
        state: AgentState::WaitingInput,
        created: now(),
    });
    snoozed.snoozed_until = now() + 600;
    let mut overflow: Vec<_> = (0..13)
        .map(|index| {
            notice_fixture(NoticeFixtureInput {
                id: &format!("extra-{index}"),
                session: "live-shell",
                state: AgentState::Completed,
                created: now(),
            })
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
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
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
    let mut waiting = notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "one",
        state: AgentState::WaitingPermission,
        created: now(),
    });
    waiting.read = true;
    app.state.notifications = vec![
        waiting,
        notice_fixture(NoticeFixtureInput {
            id: "done",
            session: "two",
            state: AgentState::Completed,
            created: now(),
        }),
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
        notice_fixture(NoticeFixtureInput {
            id: "w-early",
            session: "early",
            state: AgentState::WaitingPermission,
            created: 10,
        }),
        notice_fixture(NoticeFixtureInput {
            id: "w-late",
            session: "late",
            state: AgentState::WaitingInput,
            created: 20,
        }),
        // Older than every waiting notice, still sorted after the group.
        notice_fixture(NoticeFixtureInput {
            id: "f-failed",
            session: "failed",
            state: AgentState::Failed,
            created: 5,
        }),
        // A session with both appears once, in the waiting group.
        notice_fixture(NoticeFixtureInput {
            id: "f-early-too",
            session: "early",
            state: AgentState::Failed,
            created: 1,
        }),
    ];
    let mut dismissed = notice_fixture(NoticeFixtureInput {
        id: "dismissed",
        session: "late",
        state: AgentState::WaitingInput,
        created: 0,
    });
    dismissed.dismissed = true;
    let mut snoozed = notice_fixture(NoticeFixtureInput {
        id: "snoozed",
        session: "late",
        state: AgentState::WaitingInput,
        created: 0,
    });
    snoozed.snoozed_until = now() + 600;
    let mut resolved = notice_fixture(NoticeFixtureInput {
        id: "resolved",
        session: "late",
        state: AgentState::WaitingInput,
        created: 0,
    });
    resolved.resolved = true;
    let gone = notice_fixture(NoticeFixtureInput {
        id: "gone",
        session: "ended",
        state: AgentState::WaitingInput,
        created: 0,
    });
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
        notice_fixture(NoticeFixtureInput {
            id: "old",
            session: "s",
            state: AgentState::WaitingInput,
            created: 1,
        }),
        notice_fixture(NoticeFixtureInput {
            id: "new",
            session: "s",
            state: AgentState::WaitingInput,
            created: 2,
        }),
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
        notice_fixture(NoticeFixtureInput {
            id: "n1",
            session: "s",
            state: AgentState::WaitingInput,
            created: 1,
        }),
        notice_fixture(NoticeFixtureInput {
            id: "n2",
            session: "s",
            state: AgentState::WaitingInput,
            created: 2,
        }),
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
        notice_fixture(NoticeFixtureInput {
            id: "wait",
            session: "two",
            state: AgentState::WaitingPermission,
            created: 1,
        }),
        notice_fixture(NoticeFixtureInput {
            id: "broke",
            session: "one",
            state: AgentState::Failed,
            created: 2,
        }),
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
