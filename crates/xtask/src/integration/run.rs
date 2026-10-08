use super::super::harness::{Harness, bin, id, session, session_present, sessions};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use terminator_core::{quote as shell_quote, read_frame};
/// Fixture symlink. Windows directory/file variants are picked by the caller;
/// creating links may need developer mode or privileges at runtime.
#[cfg(unix)]
pub(super) fn symlink(original: &Path, link: &Path, dir: bool) -> Result<()> {
    let _ = dir;
    std::os::unix::fs::symlink(original, link)?;
    Ok(())
}
#[cfg(not(unix))]
pub(super) fn symlink(original: &Path, link: &Path, dir: bool) -> Result<()> {
    if dir {
        std::os::windows::fs::symlink_dir(original, link)?;
    } else {
        std::os::windows::fs::symlink_file(original, link)?;
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let a = h.project("project-a")?;
    let b = h.project("project-b")?;
    let s = h.shell(&a)?;
    let mut stream = h.attach(&s)?;
    h.write(&mut stream, "printf 'RECONNECT_PROOF_123\\n'\n")?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|text| text.contains("RECONNECT_PROOF_123"))
        },
        5,
    )?;
    drop(stream);
    symlink(
        Path::new(
            a.get("path")
                .ok_or_else(|| anyhow!("missing project path"))?
                .as_str()
                .unwrap(),
        ),
        &h.root.join("alias-a"),
        true,
    )?;
    for (path, expected) in [
        (
            PathBuf::from(
                b.get("path")
                    .ok_or_else(|| anyhow!("missing project path"))?
                    .as_str()
                    .unwrap(),
            ),
            id(&b),
        ),
        (h.root.join("alias-a"), id(&a)),
        (
            PathBuf::from(
                a.get("path")
                    .ok_or_else(|| anyhow!("missing project path"))?
                    .as_str()
                    .unwrap(),
            ),
            id(&a),
        ),
    ] {
        h.rpc(json!({"AddProject":{"path":path}}))?;
        let state = h.state()?;
        let selected = state.get("selected_project").unwrap_or(&Value::Null);
        let projects = state
            .get("projects")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing projects"))?;
        ensure!(
            selected == expected && projects.len() == 2,
            "Project selection must be idempotent: expected {expected}, selected {selected}, projects {}",
            projects.len()
        );
        h.assert_pids(std::slice::from_ref(&s))?;
    }
    for path in [h.root.join("missing"), h.root.join("daemon.log")] {
        ensure!(
            h.rpc(json!({"AddProject":{"path":path}})).is_err(),
            "Invalid project accepted"
        );
        ensure!(
            h.state()?
                .get("selected_project")
                .is_some_and(|selected| selected == id(&a)),
            "Failed open changed selection"
        );
    }
    let mut stream = h.attach(&s)?;
    h.write(
        &mut stream,
        &format!(
            "cd {}; {} cwd \"$PWD\"\n",
            shell_quote(
                b.get("path")
                    .ok_or_else(|| anyhow!("missing project path"))?
                    .as_str()
                    .unwrap(),
            ),
            shell_quote(&bin().join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(
        |st| {
            b.get("path")
                .is_some_and(|path| &session(st, id(&s))["cwd"] == path)
        },
        5,
    )?;
    ensure!(
        session(&h.state()?, id(&s))["project_id"] == id(&a),
        "cwd moved project ownership"
    );
    let event = json!({"protocol_version":1,"event_id":"fixture-1","terminal_session_id":id(&s),"agent_invocation_id":"fixture-agent","agent_kind":"custom","provider_session_id":"provider-123","state":"waiting_input","request_id":"req-1","sequence":1,"summary":"Fixture needs input","details":"Isolated test","resume":null});
    h.write(
        &mut stream,
        &format!(
            "printf '%s' {} | {} emit\n",
            shell_quote(&event.to_string()),
            shell_quote(&bin().join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(|st| st["notifications"].as_array().unwrap().len() == 1, 5)?;
    h.rpc(json!({"Focus":{"session":id(&s)}}))?;
    let st = h.state()?;
    ensure!(
        st.get("notifications")
            .and_then(|notes| notes.get(0))
            .and_then(|note| note.get("dismissed"))
            .is_some_and(|dismissed| dismissed == &json!(true))
            && st
                .get("agents")
                .and_then(|agents| agents.get(0))
                .and_then(|agent| agent.get("state"))
                .is_some_and(|agent_state| agent_state == "waiting_input"),
        "Dismissal changed agent state"
    );
    h.rpc(json!({"Hook":event}))?;
    ensure!(
        h.state()?
            .get("notifications")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing notifications"))?
            .len()
            == 1,
        "Duplicate hook"
    );
    if let Ok(mut wrong) = h.connect(
        json!({"Cwd":{"session":id(&s),"path":"/"}}),
        Some("wrong"),
        None,
    ) && let Ok(response) = read_frame::<Value>(&mut wrong)
    {
        ensure!(response.get("Error").is_some(), "Invalid auth accepted");
    }
    ensure!(
        h.state().is_ok_and(|state| {
            b.get("path")
                .is_some_and(|path| &session(&state, id(&s))["cwd"] == path)
        }),
        "Invalid auth mutated cwd"
    );
    let layout = json!({"fixture":"layout-does-not-launch-anything"});
    h.rpc(json!({"SaveLayout":{"project":id(&a),"layout":layout}}))?;
    h.write(
        &mut stream,
        "stty -icanon -echo; printf 'BLOCKING_WRITE_READY\\n'; exec sleep 30\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|t| t.lines().any(|l| l.trim() == "BLOCKING_WRITE_READY"))
        },
        5,
    )?;
    h.write(&mut stream, &"x".repeat(131072))?;
    let start = Instant::now();
    h.rpc(json!({"Stop":{"session":id(&s)}}))?;
    ensure!(
        start.elapsed() < Duration::from_secs(2),
        "Blocked paste prevented Stop"
    );
    // A plain shell has no agent resume handle, so ending it drops the record
    // and its scrollback instead of leaving a dead History entry.
    h.wait(|st| !session_present(st, id(&s)), 5)?;
    ensure!(
        h.history(id(&s)).is_err(),
        "Pruned session kept a History record"
    );
    drop(stream);
    h.restart()?;
    let state = h.state()?;
    ensure!(
        sessions(&state).is_empty(),
        "Recovery resurrected a pruned session"
    );
    ensure!(
        state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .is_some_and(|saved| saved == &layout),
        "Layout lost"
    );
    let hint = json!({
        "generation": state.get("generation").unwrap_or(&Value::Null),
        "revision": state.get("revision").unwrap_or(&Value::Null)
    });
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint.clone()))?)?
            == "Unchanged",
        "Conditional snapshot changed"
    );
    ensure!(
        h.rpc(json!("Snapshot"))?.get("State").is_some(),
        "Legacy snapshot failed"
    );
    h.project("conditional-snapshot")?;
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint.clone()))?)?
            .get("State")
            .is_some(),
        "Changed snapshot omitted"
    );
    h.restart()?;
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint))?)?
            .get("State")
            .is_some(),
        "Restart generation not invalidated"
    );
    println!(
        "{}",
        json!({"functional":"passed","reconnect_same_pid":true,"cwd_grouping":true,"hook_delivery":true,"dedup_and_dismissal":true,"restart_no_launch":true,"conditional_snapshot":true,"legacy_client":true})
    );
    Ok(())
}
