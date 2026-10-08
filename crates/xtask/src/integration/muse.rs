use super::super::harness::{Harness, output};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::path::Path;
use terminator_core::quote as shell_quote;
pub fn muse(muse: &Path) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("muse-echo")?;
    let s = h.shell(&project)?;
    let mut install = h.command("terminator-hook");
    install.args(["install", "muse"]).env(
        "TERMINATOR_CONFIG_HOME",
        project
            .get("path")
            .ok_or_else(|| anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    output(install)?;
    let args = vec![
        muse.to_string_lossy().into_owned(),
        "exec".into(),
        "--provider".into(),
        "echo".into(),
        "--workspace".into(),
        project
            .get("path")
            .ok_or_else(|| anyhow!("missing project path"))?
            .as_str()
            .unwrap()
            .into(),
        "--no-session-log".into(),
        "--no-foreign-personal-context".into(),
        "--trust-workspace".into(),
        "--disable-web-tools".into(),
        "local hook fixture".into(),
    ];
    h.write(
        &mut h.attach(&s)?,
        &(args
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
            + "\n"),
    )?;
    let state = h.wait(
        |state| {
            state["agents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["kind"] == "muse" && a["state"] == "stopped")
        },
        20,
    )?;
    ensure!(
        state
            .get("agents")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing agents"))?
            .len()
            == 1,
        "Duplicate invocation"
    );
    ensure!(
        state
            .get("notifications")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing notifications"))?
            .iter()
            .any(|n| n["state"] == "completed"),
        "Missing completion"
    );
    ensure!(
        state
            .get("agents")
            .and_then(|agents| agents.get(0))
            .and_then(|agent| agent.get("resume"))
            .and_then(|resume| resume.get("program"))
            .is_some_and(|program| program == "muse"),
        "Missing resume"
    );
    println!(
        "{}",
        json!({"muse_echo":"passed","invocations":1,"completion_received":true})
    );
    Ok(())
}
