use super::super::harness::{Harness, git, id, output};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};
/// Runs `terminator-hook ctl` against one fixture harness.
struct Cli<'a> {
    h: &'a Harness,
}
impl Cli<'_> {
    fn ctl(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut command = self.h.command("terminator-hook");
        command.arg("ctl").args(args);
        output(command)
    }
    /// Run `ctl send SESSION --stdin [extra...]` with `input` on stdin.
    fn send_stdin(&self, sid: &str, extra: &[&str], input: Vec<u8>) -> Result<()> {
        let mut send = self.h.command("terminator-hook");
        send.args(["ctl", "send", sid, "--stdin"]).args(extra);
        terminator_core::run_command(
            send,
            terminator_core::CommandOptions {
                input: Some(input),
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
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
    let cli = Cli { h: &h };
    list_and_add(&cli)?;
    let sid = create_background(&cli, &project)?;
    stdin_send_and_read(&cli, &sid)?;
    let worktrees: Value =
        serde_json::from_slice(&cli.ctl(&["worktree", "list", id(&project)])?)?;
    ensure!(
        worktrees
            .get("Worktrees")
            .and_then(Value::as_array)
            .is_some(),
        "ctl worktree list returned an unexpected reply: {worktrees}"
    );
    refusals(&cli, &sid)?;
    cursor_hooks(&h, &sid)?;
    println!(
        "{}",
        json!({"cli_list":true,"cli_add_project":true,"cli_background_create":true,"cli_stdin_send_and_read":true,"cli_worktree_list":true,"cli_refusals":true})
    );
    Ok(())
}

fn list_and_add(cli: &Cli<'_>) -> Result<()> {
    let listed: Value = serde_json::from_slice(&cli.ctl(&["list"])?)?;
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
    let extra = cli.h.root.join("cli-controls-extra");
    fs::create_dir_all(&extra)?;
    let added: Value =
        serde_json::from_slice(&cli.ctl(&["add-project", extra.to_str().unwrap_or_default()])?)?;
    ensure!(
        added
            .get("path")
            .is_some_and(|path| path == extra.to_string_lossy().as_ref()),
        "ctl add-project returned the wrong project: {added}"
    );
    Ok(())
}

/// `ctl create --background` starts a shell without a GUI; returns its ID.
fn create_background(cli: &Cli<'_>, project: &Value) -> Result<String> {
    let created: Value =
        serde_json::from_slice(&cli.ctl(&["create", id(project), "--background"])?)?;
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
    Ok(sid)
}

fn stdin_send_and_read(cli: &Cli<'_>, sid: &str) -> Result<()> {
    let h = cli.h;
    cli.send_stdin(sid, &["--enter"], b"printf 'CLI_STDIN_PROOF\\n'".to_vec())?;
    h.wait(
        |_| {
            h.history(sid)
                .is_ok_and(|text| text.contains("CLI_STDIN_PROOF"))
        },
        5,
    )?;
    ensure!(
        String::from_utf8_lossy(&cli.ctl(&["read", sid])?).contains("CLI_STDIN_PROOF"),
        "ctl read did not return saved history"
    );
    Ok(())
}

fn refusals(cli: &Cli<'_>, sid: &str) -> Result<()> {
    ensure!(
        cli.ctl(&["read", "not-a-real-session"]).is_err(),
        "ctl read accepted an unknown session"
    );
    ensure!(
        cli.send_stdin(sid, &[], vec![b'x'; 1024 * 1024 + 1])
            .is_err(),
        "ctl send accepted input above the 1 MiB limit"
    );
    Ok(())
}

/// One Cursor hook invocation: event, status, expected agent state,
/// expected notification count and expected hook reply.
type CursorCase = (
    &'static str,
    Option<&'static str>,
    &'static str,
    usize,
    &'static str,
);

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
    let cases: [CursorCase; 10] = [
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
    ];
    for (index, case) in cases.into_iter().enumerate() {
        cursor_case(h, (sid, hooks), (index, case))?;
    }
    println!(
        "Cursor hooks passed: shell delivery, prompt response, stop/sessionEnd completion/error notices, duplicate suppression, abort/close"
    );
    Ok(())
}

fn cursor_case(
    h: &Harness,
    (sid, hooks): (&str, &Value),
    (index, (event, status, expected, notices, response)): (usize, CursorCase),
) -> Result<()> {
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
    Ok(())
}
