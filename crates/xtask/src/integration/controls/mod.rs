mod checks;
mod removal;
#[cfg(all(test, unix))]
mod tests;

use super::super::harness::{Harness, git, id, output};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use terminator_core::quote as shell_quote;

// Do not put a complete readiness marker in PTY input: input echo can arrive
// before the shell executes the command. Adjacent quoted words join at runtime.
const CONTROL_SIGNAL: &str = "printf '%s\\n' 'CONTROL_SEND_''PROOF'";
const CROSS_PROJECT_SIGNAL: &str = "printf '%s\\n' 'CROSS_PROJECT_''READY'";
const CHILD_SIGNAL: &str = "printf '%s\\n' 'CHILD_''READY'";
const CHILD_STOPPED_SIGNAL: &str = "printf '%s\\n' 'CHILD_''STOPPED'";
/// Additional daemon/CLI features use a fresh instance, including live-session
/// rejection on worktree removal and OSC separation from hook state.
pub fn controls() -> Result<()> {
    let c = Checkout::create()?;
    let s = c.live_session_refuses_removal()?;
    c.cli_send_and_read(&s)?;
    let stream = c.notices(&s)?;
    c.metadata(&s)?;
    drop(stream);
    c.stop(&s)?;
    c.dirty_refusal()?;
    // Another project's shell can use this checkout, including after `cd`
    // without a directory hook and in a child whose cwd differs from its shell.
    let (outside, mut outside_stream, alias) = c.cross_project_refusal()?;
    c.descendant_refusal(&outside, &mut outside_stream)?;
    c.recorded_cwd_refusal(&alias)?;
    let branch = c.locked_refusal(&s)?;
    c.remove(&outside, &branch)?;
    c.restore()?;
    println!(
        "{}",
        json!({"worktree_registry":true,"live_and_dirty_removal_rejected":true,"cross_project_and_descendant_removal_rejected":true,"locked_removal_rejected":true,"branch_preserved":true,"cli_send_read":true,"osc_and_cli_notifications":true,"metadata":true})
    );
    Ok(())
}

/// A repository project and the task checkout `ctl worktree add` registered for it.
struct Checkout {
    h: Harness,
    project: Value,
    repo: PathBuf,
    destination: PathBuf,
    p: Value,
}

impl Checkout {
    fn create() -> Result<Self> {
        let h = Harness::new()?;
        h.setup()?;
        let project = h.project("worktree-root")?;
        let repo = PathBuf::from(
            project
                .get("path")
                .ok_or_else(|| anyhow!("missing project path"))?
                .as_str()
                .unwrap(),
        );
        git(&repo, &["init", "-q"])?;
        git(&repo, &["config", "user.name", "Fixture"])?;
        git(&repo, &["config", "user.email", "fixture@example.invalid"])?;
        fs::write(repo.join("file"), "base\n")?;
        git(&repo, &["add", "."])?;
        git(&repo, &["commit", "-qm", "base"])?;
        let destination = h.root.join("task-checkout");
        let mut command = h.command("terminator-hook");
        command
            .args(["ctl", "worktree", "add", id(&project)])
            .arg(&destination)
            .args(["--branch", "fixture-task"]);
        output(command)?;
        let state = h.state()?;
        let p = state
            .get("projects")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing projects"))?
            .iter()
            .find(|p| p["path"] == destination.to_string_lossy().as_ref())
            .context("Worktree project not registered")?
            .clone();
        ensure!(
            state
                .get("worktrees")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing worktrees"))?
                .len()
                == 1,
            "Worktree registry missing"
        );
        Ok(Self {
            h,
            project,
            repo,
            destination,
            p,
        })
    }
    fn live_session_refuses_removal(&self) -> Result<Value> {
        let h = &self.h;
        let s = h.shell(&self.p)?;
        ensure!(
            h.rpc(json!({"WorktreeRemove":{"project":id(&self.p)}}))
                .is_err(),
            "Removed a worktree with live sessions"
        );
        Ok(s)
    }
    /// Whether the first worktree record's `removed` flag equals `removed`.
    fn worktree_removed(state: &Value, removed: bool) -> bool {
        state
            .get("worktrees")
            .and_then(|worktrees| worktrees.get(0))
            .and_then(|worktree| worktree.get("removed"))
            .is_some_and(|value| value == &json!(removed))
    }
}
