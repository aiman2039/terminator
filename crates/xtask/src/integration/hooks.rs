use super::super::harness::{Harness, git, id, output};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};
/// list, and the unknown-session/oversized-input refusals. GUI-bound commands
/// (split/focus/window/browser/open-file) are covered by native fixtures.
pub fn hook_controls() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("cli-controls")?;
    git(
        Path::new(
            project
                .get("path")
                .ok_or_else(|| anyhow!("missing project path"))?
                .as_str()
                .unwrap(),
        ),
        &["init", "-q"],
    )?;

    let ctl = |args: &[&str]| -> Result<Vec<u8>> {
        let mut command = h.command("terminator-hook");
        command.arg("ctl").args(args);
        output(command)
    };

    let listed: Value = serde_json::from_slice(&ctl(&["list"])?)?;
    ensure!(
        listed.get("generation").is_some_and(Value::is_string)
            && listed
                .get("projects")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing projects"))?
                .len()
                == 1,
        "ctl list inventory incorrect: {listed}"
    );

    let extra = h.root.join("cli-controls-extra");
    fs::create_dir_all(&extra)?;
    let added: Value =
        serde_json::from_slice(&ctl(&["add-project", extra.to_str().unwrap_or_default()])?)?;
    ensure!(
        added
            .get("path")
            .is_some_and(|path| path == extra.to_string_lossy().as_ref()),
        "ctl add-project returned the wrong project: {added}"
    );

    let created: Value = serde_json::from_slice(&ctl(&["create", id(&project), "--background"])?)?;
    let sid = created
        .get("id")
        .and_then(Value::as_str)
        .context("ctl create ID")?
        .to_string();
    ensure!(
        created.get("kind").is_some_and(|kind| kind == "shell")
            && created
                .get("lifecycle")
                .is_some_and(|lifecycle| lifecycle == "running"),
        "ctl create --background did not start a shell: {created}"
    );

    let mut send = h.command("terminator-hook");
    send.args(["ctl", "send", &sid, "--stdin", "--enter"]);
    terminator_core::run_command(
        send,
        terminator_core::CommandOptions {
            input: Some(b"printf 'CLI_STDIN_PROOF\\n'".to_vec()),
            ..Default::default()
        },
    )?;
    h.wait(
        |_| {
            h.history(&sid)
                .is_ok_and(|text| text.contains("CLI_STDIN_PROOF"))
        },
        5,
    )?;

    ensure!(
        String::from_utf8_lossy(&ctl(&["read", &sid])?).contains("CLI_STDIN_PROOF"),
        "ctl read did not return saved history"
    );

    let worktrees: Value = serde_json::from_slice(&ctl(&["worktree", "list", id(&project)])?)?;
    ensure!(
        worktrees
            .get("Worktrees")
            .and_then(Value::as_array)
            .is_some(),
        "ctl worktree list returned an unexpected reply: {worktrees}"
    );

    ensure!(
        ctl(&["read", "not-a-real-session"]).is_err(),
        "ctl read accepted an unknown session"
    );
    let mut oversized = h.command("terminator-hook");
    oversized.args(["ctl", "send", &sid, "--stdin"]);
    ensure!(
        terminator_core::run_command(
            oversized,
            terminator_core::CommandOptions {
                input: Some(vec![b'x'; 1024 * 1024 + 1]),
                ..Default::default()
            },
        )
        .is_err(),
        "ctl send accepted input above the 1 MiB limit"
    );
    cursor_hooks(&h, &sid)?;
    println!(
        "{}",
        json!({"cli_list":true,"cli_add_project":true,"cli_background_create":true,"cli_stdin_send_and_read":true,"cli_worktree_list":true,"cli_refusals":true})
    );
    Ok(())
}

/// Execute the installed Cursor commands inside a real daemon-owned shell.
/// This uses fixture payloads and never starts a provider or changes user hooks.
fn cursor_hooks(h: &Harness, sid: &str) -> Result<()> {
    let home = h.root.join("cursor-home");
    let mut install = h.command("terminator-hook");
    install
        .env("TERMINATOR_CONFIG_HOME", &home)
        .args(["install", "cursor"]);
    output(install)?;
    let config: Value = serde_json::from_slice(&fs::read(home.join(".cursor/hooks.json"))?)?;
    let hooks = config.get("hooks").context("Cursor hooks")?;
    for (index, (event, status, expected, notices, response)) in [
        (
            "beforeSubmitPrompt",
            None,
            "running",
            0,
            "{\"continue\":true}",
        ),
        ("stop", Some("completed"), "completed", 1, "{}"),
        ("stop", Some("error"), "failed", 2, "{}"),
        ("stop", Some("aborted"), "unknown", 2, "{}"),
        ("sessionEnd", None, "stopped", 2, "{}"),
        // Cursor CLI ask mode emitted sessionEnd without a stop callback.
        ("sessionEnd", Some("completed"), "completed", 3, "{}"),
        ("sessionEnd", Some("completed"), "completed", 3, "{}"),
        ("sessionEnd", Some("error"), "failed", 4, "{}"),
        ("sessionEnd", Some("aborted"), "unknown", 4, "{}"),
        ("sessionEnd", Some("user_close"), "stopped", 4, "{}"),
    ]
    .into_iter()
    .enumerate()
    {
        let conversation = if event == "sessionEnd" && status.is_some() {
            "cursor-session-end-only"
        } else {
            "cursor-fixture"
        };
        let payload = json!({"hook_event_name":event,"conversation_id":conversation,"status":status,"reason":if event == "sessionEnd" { status } else { None }});
        let command = hooks
            .get(event)
            .and_then(Value::as_array)
            .and_then(|hooks| hooks.first())
            .and_then(|hook| hook.get("command"))
            .and_then(Value::as_str)
            .context("Cursor command")?;
        let reply = h.root.join(format!("cursor-{index}.reply"));
        let command = format!(
            "printf %s {} | sh -c {} > {}",
            terminator_core::quote(&payload.to_string()),
            terminator_core::quote(command),
            terminator_core::quote(&reply.to_string_lossy())
        );
        let mut send = h.command("terminator-hook");
        send.args(["ctl", "send", sid, &command, "--enter"]);
        output(send)?;
        h.wait(
            |state| {
                state["agents"].as_array().is_some_and(|agents| {
                    agents.iter().any(|agent| {
                        agent["session_id"] == sid
                            && agent["kind"] == "cursor"
                            && agent["state"] == expected
                    })
                }) && fs::read_to_string(&reply).is_ok_and(|text| text.trim() == response)
            },
            5,
        )?;
        let state = h.state()?;
        let count = state
            .get("notifications")
            .and_then(Value::as_array)
            .context("notifications")?
            .iter()
            .filter(|notice| notice["session_id"] == sid)
            .count();
        ensure!(
            count == notices,
            "Cursor {event}/{status:?} produced {count} notifications, expected {notices}"
        );
    }
    println!(
        "Cursor hooks passed: shell delivery, prompt response, stop/sessionEnd completion/error notices, duplicate suppression, abort/close"
    );
    Ok(())
}
