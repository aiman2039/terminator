use super::{
    keys::{Effect, Inbox, Key, Mode},
    rows::{Notice, Rank, rows},
};
use terminator_core::*;

fn session(id: &str, lifecycle: Lifecycle) -> Session {
    Session {
        remote: None,
        review: false,
        id: id.into(),
        project_id: "p".into(),
        label: format!("label-{id}"),
        cwd: "/p".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "g".into(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    }
}

fn agent(session: &str, state: AgentState, updated: u64) -> Agent {
    Agent {
        invocation_id: format!("i-{session}"),
        session_id: session.into(),
        kind: "codex".into(),
        provider_session_id: None,
        state,
        sequence: None,
        updated,
        resume: None,
        process: None,
    }
}

fn notification(id: &str, session: &str, state: AgentState) -> Notification {
    Notification {
        id: id.into(),
        session_id: session.into(),
        invocation_id: format!("i-{session}"),
        request_id: None,
        state,
        summary: "Needs input".into(),
        details: "Allow   cargo\ntest?".into(),
        created: 90,
        read: false,
        dismissed: false,
        resolved: false,
        snoozed_until: 0,
    }
}

fn fixture() -> State {
    let mut state = State::default();
    state.projects.push(Project {
        id: "p".into(),
        name: "proj".into(),
        path: "/p".into(),
        layout: serde_json::Value::Null,
    });
    for (id, agent_state) in [
        ("idle", None),
        ("done", Some(AgentState::Completed)),
        ("work", Some(AgentState::Running)),
        ("fail", Some(AgentState::Failed)),
        ("wait", Some(AgentState::WaitingPermission)),
    ] {
        state.sessions.push(session(id, Lifecycle::Running));
        state.agents.extend(agent_state.map(|s| agent(id, s, 50)));
    }
    state.notifications.push(notification(
        "n-wait",
        "wait",
        AgentState::WaitingPermission,
    ));
    state
        .notifications
        .push(notification("n-done", "done", AgentState::Completed));
    state
}

#[test]
fn inbox_sorts_waiting_failed_working_done_unread_then_idle() {
    let list = rows(&fixture(), 100);
    let order: Vec<_> = list.iter().map(|r| (r.session.as_str(), r.rank)).collect();
    assert_eq!(
        order,
        [
            ("wait", Rank::Waiting),
            ("fail", Rank::Failed),
            ("work", Rank::Working),
            ("done", Rank::DoneUnread),
            ("idle", Rank::Idle),
        ]
    );
    let wait = &list[0];
    assert_eq!(wait.project, "proj");
    assert_eq!(wait.age, 50);
    assert_eq!(
        wait.notice,
        Some(Notice::Agent {
            id: "n-wait".into(),
            text: "Needs input: Allow cargo test?".into()
        })
    );
}

#[test]
fn inbox_moves_a_snoozed_or_dismissed_session_to_idle() {
    let mut state = fixture();
    state.notifications[0].snoozed_until = 500;
    state.notifications[1].dismissed = true;
    let list = rows(&state, 100);
    let rank = |id: &str| list.iter().find(|r| r.session == id).unwrap().rank;
    assert_eq!(rank("wait"), Rank::Idle);
    assert_eq!(rank("done"), Rank::Idle);
    assert!(list.iter().all(|r| r.notice.is_none()));
}

#[test]
fn inbox_hides_ended_sessions_without_a_pending_notice() {
    let mut state = fixture();
    state.sessions.push(session("gone", Lifecycle::Ended));
    state.sessions.push(session("told", Lifecycle::Ended));
    state.terminal_notices.push(TerminalNotice {
        id: "t".into(),
        session_id: "told".into(),
        title: "Build".into(),
        body: "finished".into(),
        created: 1,
        dismissed: false,
    });
    let list = rows(&state, 100);
    assert!(list.iter().all(|r| r.session != "gone"));
    let told = list.iter().find(|r| r.session == "told").unwrap();
    assert_eq!(
        told.notice,
        Some(Notice::Terminal {
            id: "t".into(),
            text: "Build: finished".into()
        })
    );
}

fn typed(inbox: &mut Inbox, list: &[super::rows::Row], text: &str) -> Vec<Effect> {
    text.chars()
        .map(|c| inbox.key(Key::Char(c), list))
        .collect()
}

#[test]
fn inbox_reply_is_never_sent_without_the_confirm_key() {
    let list = rows(&fixture(), 100);
    let mut inbox = Inbox::new();
    assert_eq!(inbox.key(Key::Char('r'), &list), Effect::None);
    // Letters that are list or confirm keys only type text here.
    let effects = typed(&mut inbox, &list, "yes do it sqtd");
    assert!(effects.iter().all(|e| *e == Effect::None));
    // Enter only moves to the confirm step.
    assert_eq!(inbox.key(Key::Enter, &list), Effect::None);
    assert!(matches!(inbox.mode, Mode::Confirm { .. }));
    for key in [
        Key::Enter,
        Key::Char('y'),
        Key::Char('q'),
        Key::Down,
        Key::Backspace,
    ] {
        assert_eq!(
            inbox.key(key, &list),
            Effect::None,
            "{key:?} sent the reply"
        );
        assert!(matches!(inbox.mode, Mode::Confirm { .. }));
    }
    assert_eq!(
        inbox.key(Key::Char('s'), &list),
        Effect::Send {
            session: "wait".into(),
            text: "yes do it sqtd".into(),
            enter: true
        }
    );
}

#[test]
fn inbox_type_only_choice_sends_without_enter() {
    let list = rows(&fixture(), 100);
    let mut inbox = Inbox::new();
    inbox.key(Key::Char('r'), &list);
    typed(&mut inbox, &list, "draft");
    inbox.key(Key::Enter, &list);
    assert_eq!(
        inbox.key(Key::Char('t'), &list),
        Effect::Send {
            session: "wait".into(),
            text: "draft".into(),
            enter: false
        }
    );
}

#[test]
fn inbox_escape_discards_a_reply_and_empty_enter_does_not_confirm() {
    let list = rows(&fixture(), 100);
    let mut inbox = Inbox::new();
    inbox.key(Key::Char('r'), &list);
    assert_eq!(inbox.key(Key::Enter, &list), Effect::None);
    assert!(matches!(inbox.mode, Mode::Reply { .. }));
    typed(&mut inbox, &list, "x");
    assert_eq!(inbox.key(Key::Esc, &list), Effect::None);
    assert_eq!(inbox.mode, Mode::List);
}

#[test]
fn inbox_list_keys_dismiss_snooze_open_and_quit() {
    let list = rows(&fixture(), 100);
    let mut inbox = Inbox::new();
    assert_eq!(
        inbox.key(Key::Char('d'), &list),
        Effect::DismissAgent("n-wait".into())
    );
    assert_eq!(
        inbox.key(Key::Char('s'), &list),
        Effect::Snooze("n-wait".into())
    );
    assert_eq!(
        inbox.key(Key::Enter, &list),
        Effect::OpenScreen("wait".into())
    );
    inbox.key(Key::Down, &list);
    inbox.key(Key::Down, &list);
    // The working session has no notice: nothing to dismiss.
    assert_eq!(inbox.key(Key::Char('d'), &list), Effect::None);
    assert!(!inbox.status.is_empty());
    assert_eq!(inbox.key(Key::Char('q'), &list), Effect::Quit);
}
