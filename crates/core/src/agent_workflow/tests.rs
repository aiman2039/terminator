use super::*;
use crate::AgentState;

fn notice() -> Notification {
    Notification {
        id: "n".into(),
        session_id: "s".into(),
        invocation_id: "i".into(),
        request_id: None,
        state: AgentState::WaitingInput,
        summary: String::new(),
        details: String::new(),
        created: 1,
        read: true,
        dismissed: false,
        resolved: false,
        snoozed_until: 0,
    }
}

#[test]
fn mark_unread_clears_read_and_keeps_everything_else() {
    let mut n = notice();
    apply(&mut n, "unread", 100).unwrap();
    assert!(!n.read);
    assert!(!n.dismissed && !n.resolved);
    assert_eq!(n.state, AgentState::WaitingInput);
}

#[test]
fn snooze_for_uses_the_asked_duration_within_limits() {
    let mut n = notice();
    apply(&mut n, &snooze_for(900), 100).unwrap();
    assert_eq!(n.snoozed_until, 1000);
    assert_eq!(apply(&mut n, "snooze-for:0", 100), Err(UnknownAction));
    assert_eq!(
        apply(&mut n, &snooze_for(MAX_SNOOZE_SECS + 1), 100),
        Err(UnknownAction)
    );
    assert_eq!(apply(&mut n, "snooze-for:abc", 100), Err(UnknownAction));
}

#[test]
fn restore_undoes_dismiss_and_snooze_without_resolving() {
    let mut n = notice();
    apply(&mut n, "dismiss", 100).unwrap();
    apply(&mut n, "snooze", 100).unwrap();
    apply(&mut n, "restore", 100).unwrap();
    assert!(!n.dismissed);
    assert_eq!(n.snoozed_until, 0);
    assert!(!n.resolved);
}

#[test]
fn old_actions_keep_their_meaning() {
    let mut n = notice();
    n.read = false;
    apply(&mut n, "read", 5).unwrap();
    assert!(n.read);
    apply(&mut n, "snooze", 5).unwrap();
    assert_eq!(n.snoozed_until, 5 + DEFAULT_SNOOZE_SECS);
    assert_eq!(apply(&mut n, "launch", 5), Err(UnknownAction));
}

#[test]
fn usage_keeps_the_last_reported_model_and_mode() {
    let mut usage = AgentUsage::new(Some("opus"), None).unwrap();
    usage.merge(&AgentUsage::new(None, Some("plan")).unwrap());
    assert_eq!(usage.model.as_deref(), Some("opus"));
    assert_eq!(usage.mode_label(), Some("Plan"));
    usage.merge(&AgentUsage::new(None, Some("default")).unwrap());
    assert_eq!(usage.mode_label(), None, "the default mode shows no chip");
    assert!(AgentUsage::new(Some(" \u{7} "), None).is_none());
    let long = "m".repeat(200);
    let capped = AgentUsage::new(Some(&long), None).unwrap();
    assert_eq!(capped.model.unwrap().len(), 64);
}

#[test]
fn restore_seed_ends_lines_with_crlf_and_marks_the_end() {
    let seed = restore_seed("one\ntwo\r\n\n");
    assert!(seed.starts_with("one\r\ntwo\r\n"));
    assert!(seed.contains("Saved output"));
    assert!(restore_seed("  \n").is_empty());
}

#[test]
fn restore_seed_keeps_only_the_tail_from_a_line_start() {
    let long = format!("{}\nlast line\n", "x".repeat(MAX_RESTORE_SEED + 10));
    let seed = restore_seed(&long);
    assert!(seed.starts_with("last line\r\n"));
}
