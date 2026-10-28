use super::super::super::harness::{Harness, git, id, require_first_layout, session, sessions};
use super::super::dispatch::{CaptureInput, Options, plain, prefs, setup};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, thread, time::Duration};

pub(crate) fn terminal_actions(o: &Options) -> Result<()> {
    let (h, p, s, root) = setup("terminal-actions")?;
    let source = root.join("hello.rs");
    seed_git(&root, &source)?;
    retain_ended_shell(&h, &p)?;
    history_tool(&h, o, &p)?;
    git_open(&h, o)?;
    h.layout(&p, &s)?;
    hover_editor(&h, o, (&s, &source))?;
    lower_split(&h, o)?;
    h.assert_pids(&s)
}

fn seed_git(root: &Path, source: &Path) -> Result<()> {
    fs::write(source, "fn main() {}\n")?;
    git(root, &["init", "-q"])?;
    git(root, &["add", "hello.rs"])?;
    fs::write(source, "fn changed() {}\n")?;
    fs::write(root.join("README.md"), "untracked\n")?;
    Ok(())
}

fn retain_ended_shell(h: &Harness, p: &Value) -> Result<()> {
    let ended = h.shell(p)?;
    // History keeps resumable sessions. Plain ended shells are pruned.
    // Record fixture metadata only; no agent process is launched.
    h.rpc(json!({"Hook": {
        "protocol_version":1,"event_id":"native-history-retain",
        "terminal_session_id":id(&ended),"agent_invocation_id":"native-history",
        "agent_kind":"custom","provider_session_id":"native-history",
        "state":"waiting_input","request_id":"native-history","sequence":1,
        "summary":"History fixture","details":"",
        "resume":{"program":"codex","args":["resume","native-history"]}
    }}))?;
    h.rpc(json!({"Stop":{"session":id(&ended)}}))?;
    h.wait(|st| session(st, id(&ended))["lifecycle"] == "ended", 5)?;
    Ok(())
}

fn history_tool(h: &Harness, o: &Options, p: &Value) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "history",
        actions: json!([{"at_ms":900,"target":"header-overflow"},{"at_ms":1100,"target":"tool-History"}]),
        after: 2500,
    })?;
    ensure!(
        prefs(h)?.get("tool").is_some_and(|tool| tool == "history"),
        "Global History selection not saved"
    );
    plain(CaptureInput {
        h,
        opts: o,
        name: "history-collapsed",
        actions: json!([
            {"at_ms":1000,"target":format!("history-project:{}",id(p))}
        ]),
        after: 2000,
    })?;
    ensure!(
        history_expanded(h, p)? == Some(false),
        "History collapse not saved"
    );
    plain(CaptureInput {
        h,
        opts: o,
        name: "history-expanded",
        actions: json!([
            {"at_ms":1000,"target":format!("history-project:{}",id(p))}
        ]),
        after: 2000,
    })?;
    ensure!(
        history_expanded(h, p)? == Some(true),
        "History expansion not saved"
    );
    Ok(())
}

fn history_expanded(h: &Harness, p: &Value) -> Result<Option<bool>> {
    Ok(prefs(h)?
        .get("history_expanded")
        .and_then(|expanded| expanded.get(id(p)))
        .and_then(Value::as_bool))
}

fn git_open(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "git-open",
        actions: json!([{"at_ms":1000,"target":"tool-Git"},{"at_ms":1900,"target":"git-file-README.md"}]),
        after: 3500,
    })?;
    let git_state = h.state()?;
    let layout = require_first_layout(&git_state)?.to_string();
    ensure!(
        layout.contains("README.md") && layout.contains("Diff"),
        "Git file click did not open a diff: {layout}"
    );
    ensure!(
        sessions(&git_state).iter().all(|s| s["file"]
            .as_str()
            .is_none_or(|path| !path.ends_with("README.md"))),
        "Git file click opened an editor"
    );
    Ok(())
}

fn hover_editor(h: &Harness, o: &Options, (s, source): (&[Value], &Path)) -> Result<()> {
    for session in s {
        h.write(
            &mut h.attach(session)?,
            "printf '\\033[2J\\033[H./hello.rs:2\\r\\n'\n",
        )?;
        thread::sleep(Duration::from_millis(50));
    }
    plain(CaptureInput {
        h,
        opts: o,
        name: "hover-editor",
        actions: json!([{"at_ms":1100,"target":"terminal","hover":true},{"at_ms":1700,"target":"terminal","hover_offset":[45,10]},{"at_ms":2400,"target":"Open in editor split"}]),
        after: 4000,
    })?;
    ensure!(
        sessions(&h.state()?)
            .iter()
            .any(|s| s["kind"] == "editor" && s["file"] == source.to_string_lossy().as_ref()),
        "Terminal path hover did not open file"
    );
    Ok(())
}

fn lower_split(h: &Harness, o: &Options) -> Result<()> {
    let before = sessions(&h.state()?).len();
    plain(CaptureInput {
        h,
        opts: o,
        name: "lower-split",
        actions: json!([{"at_ms":1100,"target":"terminal","right_click":true},{"at_ms":2000,"target":"Split down"}]),
        after: 3500,
    })?;
    ensure!(
        sessions(&h.state()?).len() == before.saturating_add(1),
        "Lower pane split failed"
    );
    Ok(())
}
