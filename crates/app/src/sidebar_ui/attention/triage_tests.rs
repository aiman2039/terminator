use super::*;
use crate::app::tests::support::{NoticeFixtureInput, fixture, notice_fixture, session_fixture};
use crate::{After, Job};
use std::sync::mpsc;
use terminator_core::{Request, SessionKind, agent_workflow};

fn inbox(
    triage: bool,
) -> (
    App,
    eframe::egui::Context,
    tempfile::TempDir,
    mpsc::Receiver<Job>,
) {
    let (mut app, ctx, dir) = fixture();
    app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
    app.state.notifications = vec![notice_fixture(NoticeFixtureInput {
        id: "wait",
        session: "live-shell",
        state: AgentState::WaitingPermission,
        created: now(),
    })];
    if triage {
        app.state.capabilities = vec![
            agent_workflow::TRIAGE_CAPABILITY.into(),
            terminator_core::SCREEN_CAPABILITY.into(),
        ];
    }
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    (app, ctx, dir, received)
}

fn requests(received: &mpsc::Receiver<Job>) -> Vec<Request> {
    let mut out = Vec::new();
    while let Ok(job) = received.try_recv() {
        if let Job::Control(request, _) = job {
            out.push(*request);
        }
    }
    out
}

fn notice_actions(received: &mpsc::Receiver<Job>) -> Vec<String> {
    requests(received)
        .into_iter()
        .filter_map(|request| match request {
            Request::Notice { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

fn group(app: &App) -> NoticeGroup {
    NoticeGroup {
        notices: app.state.notifications.clone(),
    }
}

#[test]
fn mark_unread_sends_unread_and_keeps_the_notice_pending() {
    let (mut app, _ctx, _dir, received) = inbox(true);
    app.state.notifications[0].read = true;
    app.apply_group_action(&group(&app), AttentionAction::Unread);
    assert!(!app.state.notifications[0].read);
    assert!(!app.state.notifications[0].dismissed);
    assert_eq!(notice_actions(&received), ["unread"]);
}

#[test]
fn mark_unread_is_not_sent_to_an_older_daemon() {
    let (mut app, _ctx, _dir, received) = inbox(false);
    app.state.notifications[0].read = true;
    app.apply_group_action(&group(&app), AttentionAction::Unread);
    assert!(app.state.notifications[0].read);
    assert!(notice_actions(&received).is_empty());
}

#[test]
fn timed_snooze_uses_the_picked_duration() {
    let (mut app, _ctx, _dir, received) = inbox(true);
    app.apply_group_action(&group(&app), AttentionAction::SnoozeFor(3600));
    let until = app.state.notifications[0].snoozed_until;
    assert!(until >= now() + 3590 && until <= now() + 3600);
    assert_eq!(notice_actions(&received), ["snooze-for:3600"]);
}

#[test]
fn timed_snooze_falls_back_to_ten_minutes_on_an_older_daemon() {
    let (mut app, _ctx, _dir, received) = inbox(false);
    app.apply_group_action(&group(&app), AttentionAction::SnoozeFor(3600));
    assert!(app.state.notifications[0].snoozed_until <= now() + 600);
    assert_eq!(notice_actions(&received), ["snooze"]);
    assert!(app.triage.undo.is_none(), "an older daemon cannot undo");
}

#[test]
fn undo_after_dismiss_restores_the_notice_without_focusing() {
    let (mut app, _ctx, _dir, received) = inbox(true);
    app.apply_group_action(&group(&app), AttentionAction::Dismiss);
    assert!(app.state.notifications[0].dismissed);
    assert!(app.triage.undo.is_some());
    app.undo_triage();
    assert!(!app.state.notifications[0].dismissed);
    assert_eq!(app.state.notifications[0].snoozed_until, 0);
    assert!(app.triage.undo.is_none());
    assert!(app.active_session.is_none());
    assert_eq!(notice_actions(&received), ["dismiss", "restore"]);
}

#[test]
fn pin_moves_a_session_first_and_survives_only_known_sessions() {
    let (mut app, _ctx, _dir, received) = inbox(true);
    app.state
        .sessions
        .push(session_fixture("other", SessionKind::Shell));
    app.preferences
        .pinned_agent_sessions
        .insert("gone".to_owned());
    app.apply_group_action(&group(&app), AttentionAction::Pin);
    assert!(app.session_pinned("live-shell"));
    assert!(!app.session_pinned("gone"));
    let ids = ["other", "live-shell"];
    let ordered = app.pinned_first(&ids, |id| Some(*id));
    assert_eq!(ordered, [&"live-shell", &"other"]);
    assert!(requests(&received).is_empty(), "pin is a GUI preference");
}

#[test]
fn peek_reads_the_screen_and_sends_nothing_to_the_pty() {
    let (mut app, _ctx, _dir, received) = inbox(true);
    assert!(app.terminal_input_enabled("live-shell"));
    app.apply_group_action(&group(&app), AttentionAction::Peek);
    assert_eq!(app.triage.peek.as_deref(), Some("live-shell"));
    let mut sent = Vec::new();
    while let Ok(job) = received.try_recv() {
        match job {
            Job::Control(request, After::Text(key)) => sent.push((*request, key)),
            _ => panic!("peek sent an unexpected job"),
        }
    }
    assert!(matches!(
        sent.as_slice(),
        [(Request::Screen { session }, key)] if session == "live-shell" && key == "peek:live-shell"
    ));
    // Every terminal input gate is closed while the peek shows.
    assert!(!app.terminal_input_enabled("live-shell"));
    assert!(!app.strip_terminal_input_enabled("live-shell"));
    assert!(!app.float_terminal_input_enabled("live-shell"));
    app.close_session_peek();
    assert!(app.terminal_input_enabled("live-shell"));
}

#[test]
fn escape_closes_the_peek_and_reaches_no_terminal() {
    let (mut app, ctx, _dir, received) = inbox(true);
    app.open_session_peek("live-shell");
    let _ = requests(&received);
    let mut output = ctx.run_ui(
        eframe::egui::RawInput {
            events: vec![crate::app::tests::support::key_press(
                eframe::egui::Key::Escape,
            )],
            ..Default::default()
        },
        |ui| app.session_peek_modal(ui.ctx()),
    );
    output.textures_delta.clear();
    assert!(app.triage.peek.is_none());
    assert!(requests(&received).is_empty());
}

#[test]
fn snooze_until_tomorrow_is_between_one_and_thirty_three_hours() {
    let secs = triage::seconds_until_tomorrow(now());
    assert!((60..=33 * 60 * 60).contains(&secs), "{secs}");
}
