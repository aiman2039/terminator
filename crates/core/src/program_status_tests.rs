use super::*;

fn report(payload: &str) -> Option<Report> {
    let parts: Vec<&[u8]> = std::iter::once(b"7501".as_slice())
        .chain(payload.split(';').map(str::as_bytes))
        .collect();
    parse(&parts)
}

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

fn set(payload: &str) -> ProgramStatus {
    match report(payload) {
        Some(Report::Set(status)) => status,
        other => panic!("{payload}: {other:?}"),
    }
}

#[test]
fn a_full_report_keeps_every_field() {
    let status = set(&format!(
        "state=blocked:kind=permission:progress=40:app=claude-code:title={}:msg={}",
        b64("Deploy"),
        b64("Approve the rollout")
    ));
    assert_eq!(status.state, ProgramState::Blocked);
    assert_eq!(status.kind.as_deref(), Some("permission"));
    assert_eq!(status.progress, Some(40));
    assert_eq!(status.app.as_deref(), Some("claude-code"));
    assert_eq!(status.title.as_deref(), Some("Deploy"));
    assert_eq!(status.message.as_deref(), Some("Approve the rollout"));
    assert_eq!(badge(&status), "claude-code: waiting 40%");
    assert_eq!(
        notice(&status),
        Some((
            "claude-code is waiting (permission)".to_owned(),
            "Approve the rollout".to_owned()
        ))
    );
}

#[test]
fn queries_clears_and_unknown_keys_follow_the_spec() {
    assert_eq!(report("?"), Some(Report::Query));
    assert_eq!(report("state=clear"), Some(Report::Clear));
    let status = set("state=working:color=red:state=done:bad pair:app=x");
    assert_eq!(status.state, ProgramState::Done, "the last value wins");
    assert_eq!(status.progress, None);
    assert_eq!(notice(&set("state=working")), None);
    assert_eq!(notice(&set("state=done")).unwrap().0, "Program is done");
    // Kind only with blocked, progress only with working or blocked.
    assert_eq!(set("state=done:kind=auth:progress=10").kind, None);
}

#[test]
fn malformed_and_oversized_reports_are_discarded() {
    for payload in [
        "",
        "state=",
        "state=sleeping",
        "app=x",
        "state=working:progress=101",
        "state=working:progress=-1",
        "state=working:id=child",
        "state=clear:id=child",
        "state=done:msg=A",
        &format!("state=done:app={}", "a".repeat(33)),
        &format!("state=done:title={}", b64(&"t".repeat(193))),
        &format!("state=done:msg={}", b64("bell\u{7}")),
    ] {
        assert_eq!(report(payload), None, "{payload:?}");
    }
    let huge = format!("state=done:msg={}", "A".repeat(64 * 1024));
    assert_eq!(report(&huge), None);
    assert_eq!(parse(&[b"7502".as_slice(), b"state=done"]), None);
    // Bad bytes in a value skip that pair only.
    assert_eq!(set("state=done:app=a b").app, None);
}

#[test]
fn invisible_formatting_is_stripped_from_text() {
    let status = set(&format!(
        "state=done:msg={}",
        b64("ok\u{202E}gnp.exe\u{200B}")
    ));
    assert_eq!(status.message.as_deref(), Some("okgnp.exe"));
}
