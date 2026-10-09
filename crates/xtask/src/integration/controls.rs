use super::super::harness::{Harness, git, id, output, session, session_present};
use super::run::symlink;
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
    let s = h.shell(&p)?;
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Removed a worktree with live sessions"
    );
    let mut send = h.command("terminator-hook");
    send.args(["ctl", "send", id(&s), CONTROL_SIGNAL, "--enter"]);
    output(send)?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|s| s.contains("CONTROL_SEND_PROOF"))
        },
        5,
    )?;
    let mut read = h.command("terminator-hook");
    read.args(["ctl", "read", id(&s), "--screen"]);
    ensure!(
        String::from_utf8_lossy(&output(read)?).contains("CONTROL_SEND_PROOF"),
        "Screen read did not return live content"
    );
    let mut notify = h.command("terminator-hook");
    notify.args([
        "ctl",
        "notify",
        id(&s),
        "CLI notification",
        "--title",
        "Fixture",
    ]);
    output(notify)?;
    let mut stream = h.attach(&s)?;
    h.write(&mut stream, "printf '\\033]9;OSC fixture\\007'\n")?;
    let state = h.wait(|s| s["terminal_notices"].as_array().unwrap().len() >= 2, 5)?;
    ensure!(
        state
            .get("agents")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing agents"))?
            .is_empty()
            && state
                .get("notifications")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing notifications"))?
                .is_empty(),
        "Terminal notices changed agent lifecycle"
    );
    let notice = state
        .get("terminal_notices")
        .and_then(|notices| notices.get(0))
        .and_then(|notice| notice.get("id"))
        .ok_or_else(|| anyhow!("missing terminal notice"))?
        .as_str()
        .unwrap();
    let mut dismiss = h.command("terminator-hook");
    dismiss.args(["ctl", "dismiss-notice", notice]);
    output(dismiss)?;
    ensure!(
        h.state()?
            .get("terminal_notices")
            .and_then(|notices| notices.get(0))
            .and_then(|notice| notice.get("dismissed"))
            .is_some_and(|dismissed| dismissed == &json!(true)),
        "Notice dismissal failed"
    );
    let stub = h.root.join("gh-fixture");
    fs::create_dir(&stub)?;
    symlink(&std::env::current_exe()?, &stub.join("gh"), false)?;
    let mut metadata = h.command("terminator-hook");
    metadata.args(["ctl", "metadata", id(&s)]);
    metadata.env("PATH",format!("{}:{}",stub.display(),std::env::var("PATH").unwrap_or_default())).env("TERMINATOR_FIXTURE_GH_JSON",json!({"number":7,"title":"Fixture PR","url":"https://github.com/example/fixture/pull/7","state":"OPEN"}).to_string()).arg("--pr");
    let metadata: Value = serde_json::from_slice(&output(metadata)?)?;
    ensure!(
        metadata
            .get("branch")
            .is_some_and(|branch| branch == "fixture-task")
            && metadata
                .get("worktree")
                .is_some_and(|worktree| worktree == &json!(true)),
        "Worktree metadata incorrect: {metadata}"
    );
    ensure!(
        metadata
            .get("pull_request")
            .and_then(|pull_request| pull_request.get("number"))
            .is_some_and(|number| number == &json!(7)),
        "PR metadata fixture failed: {metadata}"
    );
    drop(stream);
    h.rpc(json!({"Stop":{"session":id(&s)}}))?;
    h.wait(|st| !session_present(st, id(&s)), 5)?;
    fs::write(destination.join("dirty"), "keep")?;
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Dirty checkout was removed"
    );
    fs::remove_file(destination.join("dirty"))?;
    // Another project's shell can use this checkout, including after `cd`
    // without a directory hook and in a child whose cwd differs from its shell.
    let outside = h.shell(&project)?;
    let mut outside_stream = h.attach(&outside)?;
    let alias = h.root.join("checkout-alias");
    symlink(&destination, &alias, true)?;
    h.write(
        &mut outside_stream,
        &format!(
            "stty -echo; cd {} && {CROSS_PROJECT_SIGNAL}\n",
            shell_quote(&alias.to_string_lossy())
        ),
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CROSS_PROJECT_READY"))
        },
        5,
    )?;
    ensure!(
        h.state().is_ok_and(|state| {
            project
                .get("path")
                .is_some_and(|path| &session(&state, id(&outside))["cwd"] == path)
        }),
        "Fixture unexpectedly reported cwd"
    );
    let error = h
        .rpc(json!({"WorktreeRemove":{"project":id(&p)}}))
        .expect_err("Removed checkout with another project's process inside");
    ensure!(
        error.to_string().contains("Stop processes using"),
        "Wrong cwd refusal: {error:#}"
    );
    ensure!(
        destination.join("file").is_file()
            && h.state()?
                .get("worktrees")
                .and_then(|worktrees| worktrees.get(0))
                .and_then(|worktree| worktree.get("removed"))
                .is_some_and(|removed| removed == &json!(false)),
        "Refusal changed checkout or registry"
    );
    h.write(
        &mut outside_stream,
        &format!(
            "cd {} && {{ (cd {} && {CHILD_SIGNAL} && exec sleep 30) & }}\n",
            shell_quote(&repo.to_string_lossy()),
            shell_quote(&destination.to_string_lossy())
        ),
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CHILD_READY"))
        },
        5,
    )?;
    let error = h
        .rpc(json!({"WorktreeRemove":{"project":id(&p)}}))
        .expect_err("Removed checkout with a live descendant inside");
    ensure!(
        error.to_string().contains("Stop processes using"),
        "Wrong descendant refusal: {error:#}"
    );
    h.write(
        &mut outside_stream,
        &format!("kill %1; wait; {CHILD_STOPPED_SIGNAL}\n"),
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CHILD_STOPPED"))
        },
        5,
    )
    .with_context(|| format!("Child cleanup history: {:?}", h.history(id(&outside))))?;
    h.assert_pids(std::slice::from_ref(&outside))?;
    let recorded = h
        .rpc(json!({"Create":{"project":id(&project),"cwd":alias,"file":null,"line":null,"editor":false}}))?
        .get("Created")
        .cloned()
        .ok_or_else(|| anyhow!("missing Created"))?;
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Removed checkout used by another project's recorded cwd"
    );
    h.rpc(json!({"Stop":{"session":id(&recorded)}}))?;
    h.wait(|st| !session_present(st, id(&recorded)), 5)?;
    let branch = git(&repo, &["rev-parse", "refs/heads/fixture-task"])?;
    ensure!(
        git(&destination, &["status", "--porcelain"])?.is_empty(),
        "Lock fixture is dirty"
    );
    git(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "fixture lock",
            destination.to_str().unwrap(),
        ],
    )?;
    let mut locked_remove = h.command("terminator-hook");
    locked_remove.args(["ctl", "worktree", "remove", id(&p)]);
    let error = output(locked_remove).expect_err("Locked checkout was removed");
    ensure!(
        format!("{error:#}").contains("locked"),
        "Wrong refusal: {error:#}"
    );
    ensure!(
        fs::read(destination.join("file"))? == b"base\n",
        "Locked tracked file changed"
    );
    let common = terminator_core::worktrees::common_dir(&repo)?;
    ensure!(
        terminator_core::worktrees::list(&common)?
            .iter()
            .any(|w| w.path == destination && w.locked),
        "Git registration or lock lost"
    );
    ensure!(
        git(&repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
        "Locked branch reference changed"
    );
    let state = h.state()?;
    ensure!(
        state
            .get("worktrees")
            .and_then(|worktrees| worktrees.get(0))
            .and_then(|worktree| worktree.get("removed"))
            .is_some_and(|removed| removed == &json!(false)),
        "Refusal marked checkout removed"
    );
    ensure!(
        !session_present(&state, id(&s)),
        "Lock refusal depended on a live session"
    );
    git(
        &repo,
        &["worktree", "unlock", destination.to_str().unwrap()],
    )?;
    let mut remove = h.command("terminator-hook");
    remove.args(["ctl", "worktree", "remove", id(&p)]);
    output(remove)?;
    h.assert_pids(std::slice::from_ref(&outside))?;
    ensure!(
        !destination.exists()
            && h.state()?
                .get("worktrees")
                .and_then(|worktrees| worktrees.get(0))
                .and_then(|worktree| worktree.get("removed"))
                .is_some_and(|removed| removed == &json!(true)),
        "Worktree removal not recorded"
    );
    ensure!(
        git(&repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
        "Removal changed branch reference"
    );
    ensure!(
        h.state()?
            .get("selected_project")
            .is_some_and(|selected| selected != id(&p)),
        "Removed worktree remains selected"
    );
    ensure!(
        h.rpc(json!({"SelectProject":{"project":id(&p)}})).is_err(),
        "Removed worktree can be selected"
    );
    ensure!(
        h.rpc(json!({"Create":{"project":id(&p),"cwd":repo,"editor":false}}))
            .is_err(),
        "Removed worktree can create a session in another directory"
    );
    // Explicitly opening a restored folder must not inherit the removal marker.
    fs::create_dir(&destination)?;
    h.rpc(json!({"AddProject":{"path":destination}}))?;
    ensure!(
        h.state()?
            .get("selected_project")
            .is_some_and(|selected| selected == id(&p)),
        "Restored project was not selected"
    );
    let restored = h.shell(&p)?;
    ensure!(
        restored
            .get("cwd")
            .is_some_and(|cwd| cwd == destination.to_string_lossy().as_ref()),
        "Restored shell used a stale folder"
    );
    println!(
        "{}",
        json!({"worktree_registry":true,"live_and_dirty_removal_rejected":true,"cross_project_and_descendant_removal_rejected":true,"locked_removal_rejected":true,"branch_preserved":true,"cli_send_read":true,"osc_and_cli_notifications":true,"metadata":true})
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn readiness_markers_require_execution_and_cannot_match_input_echo() {
        for (command, marker) in [
            (CONTROL_SIGNAL, "CONTROL_SEND_PROOF"),
            (CROSS_PROJECT_SIGNAL, "CROSS_PROJECT_READY"),
            (CHILD_SIGNAL, "CHILD_READY"),
            (CHILD_STOPPED_SIGNAL, "CHILD_STOPPED"),
        ] {
            assert!(!command.contains(marker), "Input echo could match {marker}");
            let output = Command::new("sh").args(["-c", command]).output().unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{marker}\n")
            );
        }
    }

    #[test]
    fn failed_directory_change_does_not_emit_readiness() {
        let dir = tempfile::tempdir().unwrap();
        let command = format!(
            "cd {} && {CROSS_PROJECT_SIGNAL}",
            shell_quote(&dir.path().join("missing").to_string_lossy())
        );
        let output = Command::new("sh").args(["-c", &command]).output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("CROSS_PROJECT_READY"));
    }
}
