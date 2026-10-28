use crate::harness::{Harness, id, session_present, sessions, wait_child};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    thread,
    time::{Duration, Instant},
};
use terminator_core::{quote, transport};

/// Two shells, the first printing `marker` into its history and then sleeping.
/// Returns both shells and the first shell's attached stream.
fn marked_shells(
    h: &Harness,
    project: &Value,
    marker: &str,
) -> Result<(Value, Value, transport::Stream)> {
    let first = h.shell(project)?;
    let second = h.shell(project)?;
    let mut stream = h.attach(&first)?;
    h.write(&mut stream, &format!("printf '\\n{marker}\\n'; sleep 60\n"))?;
    h.wait(
        |_| h.history(id(&first)).is_ok_and(|s| s.contains(marker)),
        5,
    )?;
    Ok((first, second, stream))
}

/// A shell that ignores SIGHUP; touches `ready_name` once the trap is set.
fn stubborn_shell(
    h: &Harness,
    project: &Value,
    ready_name: &str,
) -> Result<(Value, transport::Stream)> {
    let stubborn = h.shell(project)?;
    let mut stream = h.attach(&stubborn)?;
    let ready = h.root.join(ready_name);
    h.write(
        &mut stream,
        &format!("trap '' HUP; touch {}\n", quote(&ready.to_string_lossy())),
    )?;
    h.wait(|_| ready.exists(), 5)?;
    Ok((stubborn, stream))
}

fn generation(h: &Harness) -> Result<Value> {
    h.state()?
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow!("missing generation"))
}

/// Run `terminator-hook ctl shutdown` with `args`.
fn shutdown(h: &Harness, args: &[&str]) -> Result<Output> {
    Ok(h.command("terminator-hook")
        .args(["ctl", "shutdown"])
        .args(args)
        .output()?)
}

/// Wait for the stopped daemon, start a fresh one, and return its snapshot.
fn restart_after_cleanup(h: &mut Harness) -> Result<Value> {
    wait_child(&mut h.daemon.as_mut().unwrap().0, Duration::from_secs(5))?;
    h.daemon.take();
    h.start()?;
    h.state()
}

/// Whether a ctl shutdown JSON reply reports `shutdown` and two stopped sessions.
fn stopped_two(result: &Value) -> bool {
    result
        .get("shutdown")
        .is_some_and(|shutdown| shutdown.as_bool() == Some(true))
        && result
            .get("stopped_sessions")
            .is_some_and(|stopped| stopped.as_u64() == Some(2))
}

pub(super) fn clean_shutdown() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("stop-all")?;
    let (first, second, _first_stream) = marked_shells(&h, &project, "CLEANUP_HISTORY_MARKER")?;
    let helper = h
        .state()?
        .get("attachment_helper_executable")
        .ok_or_else(|| anyhow!("missing attachment helper"))?
        .as_str()
        .unwrap()
        .to_owned();
    refused_shutdowns(&h, &first, &second)?;
    let output = shutdown(&h, &["--stop-all", "--timeout", "5"])?;
    ensure!(
        output.status.success(),
        "Cleanup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(stopped_two(&result), "Cleanup did not report both sessions");
    ensure!(
        !std::path::Path::new(&helper).exists()
            && !terminator_core::Paths::at(h.root.clone()).socket().exists(),
        "Cleanup returned before daemon teardown"
    );
    let state = restart_after_cleanup(&mut h)?;
    ensure!(
        sessions(&state).is_empty(),
        "Cleanup retained records for non-resumable exited sessions"
    );
    ensure!(
        h.history(id(&first)).is_err(),
        "Cleanup kept pruned terminal history"
    );
    cleanup_timeout_keeps_daemon(&h, &project)?;
    println!(
        "{}",
        json!({"stop_all_cleanup":true,"default_and_self_shutdown_refused":true,"history_pruned":true,"timeout_preserves_daemon":true})
    );
    Ok(())
}

/// Shutdown refuses live sessions by default and from inside a Terminator terminal.
fn refused_shutdowns(h: &Harness, first: &Value, second: &Value) -> Result<()> {
    let refused = shutdown(h, &[])?;
    ensure!(
        !refused.status.success()
            && String::from_utf8_lossy(&refused.stderr).contains("--stop-all"),
        "Default shutdown must refuse live sessions"
    );
    h.assert_pids(&[first.clone(), second.clone()])?;
    let inside = h
        .command("terminator-hook")
        .env("TERMINATOR_SESSION_ID", id(first))
        .args(["ctl", "shutdown", "--stop-all"])
        .output()?;
    ensure!(
        !inside.status.success()
            && String::from_utf8_lossy(&inside.stderr).contains("outside Terminator"),
        "Cleanup must not kill its own terminal"
    );
    h.assert_pids(&[first.clone(), second.clone()])
}

// A process that ignores the normal stop signal must not cause a fake success.
fn cleanup_timeout_keeps_daemon(h: &Harness, project: &Value) -> Result<()> {
    let (stubborn, mut stream) = stubborn_shell(h, project, "ignoring-hup")?;
    let generation = generation(h)?;
    let output = shutdown(h, &["--stop-all", "--timeout", "1"])?;
    // Always release the fixture's HUP-ignoring shell before checking assertions.
    h.write(&mut stream, "exit\n")?;
    h.wait(|s| !session_present(s, id(&stubborn)), 5)?;
    ensure!(
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("did not stop"),
        "Cleanup falsely succeeded for a refusing process"
    );
    ensure!(
        h.state()?
            .get("generation")
            .is_some_and(|current| current == &generation),
        "Timed-out cleanup replaced the daemon"
    );
    Ok(())
}

fn relaunch_stub(h: &Harness) -> Result<PathBuf> {
    let stub = h.root.join("relaunch-stub");
    // Rename into place. A direct redirect exists before the writes finish,
    // and the fixture would observe an empty marker.
    fs::write(
        &stub,
        "#!/bin/sh\n{\n  printf 'data=%s\\n' \"$TERMINATOR_DATA_DIR\"\n  printf 'runtime=%s\\n' \"$TERMINATOR_RUNTIME_DIR\"\n  printf 'session=%s\\n' \"${TERMINATOR_SESSION_ID-}\"\n} > \"$TERMINATOR_DATA_DIR/relaunched.tmp\"\nmv \"$TERMINATOR_DATA_DIR/relaunched.tmp\" \"$TERMINATOR_DATA_DIR/relaunched\"\n",
    )?;
    #[cfg(unix)]
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700))?;
    Ok(stub)
}

pub(super) fn wait_relaunch_marker(root: &Path, timeout: Duration) -> Result<String> {
    let marker = root.join("relaunched");
    let end = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("deadline overflow"))?;
    loop {
        if marker.is_file() {
            let text = fs::read_to_string(&marker)?;
            if text.contains("session=") {
                return Ok(text);
            }
        }
        ensure!(
            Instant::now() < end,
            "Relaunch stub did not complete: {} (restart.log: {})",
            marker.display(),
            fs::read_to_string(root.join("restart.log")).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(30));
    }
}

/// `ctl shutdown --relaunch --exe STUB` with the given timeout.
fn relaunch(h: &Harness, stub: &Path, timeout: &str) -> Result<Output> {
    Ok(h.command("terminator-hook")
        .args([
            "ctl",
            "shutdown",
            "--stop-all",
            "--timeout",
            timeout,
            "--relaunch",
            "--exe",
        ])
        .arg(stub)
        .output()?)
}

pub(super) fn relaunch_shutdown() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("relaunch")?;
    let (first, _second, _first_stream) = marked_shells(&h, &project, "RELAUNCH_HISTORY_MARKER")?;
    let stub = relaunch_stub(&h)?;
    let output = relaunch(&h, &stub, "5")?;
    ensure!(
        output.status.success(),
        "Relaunch cleanup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(
        stopped_two(&result)
            && result
                .get("relaunched")
                .is_some_and(|relaunched| relaunched.as_bool() == Some(true)),
        "Relaunch did not report both sessions"
    );
    let marker = wait_relaunch_marker(&h.root, Duration::from_secs(5))?;
    let socket = h.root.join("run/daemon.sock");
    ensure!(
        marker.contains(&format!("data={}", h.root.display()))
            && marker.contains(&format!("runtime={}", h.root.join("run").display()))
            && marker.contains("session=\n")
            && !socket.exists(),
        "Relaunch stub missed isolated env or ran before teardown (socket_exists={}, marker={marker:?})",
        socket.exists()
    );
    let state = restart_after_cleanup(&mut h)?;
    ensure!(
        sessions(&state).is_empty(),
        "Relaunch retained records for non-resumable exited sessions"
    );
    ensure!(
        h.history(id(&first)).is_err(),
        "Relaunch kept pruned terminal history"
    );
    relaunch_timeout_keeps_daemon(&h, &project, &stub)?;
    println!(
        "{}",
        json!({"relaunch_cleanup":true,"history_pruned":true,"timeout_preserves_daemon":true,"stub_relaunched":true})
    );
    Ok(())
}

fn relaunch_timeout_keeps_daemon(h: &Harness, project: &Value, stub: &Path) -> Result<()> {
    let (stubborn, mut stream) = stubborn_shell(h, project, "ignoring-hup-relaunch")?;
    let generation = generation(h)?;
    fs::remove_file(h.root.join("relaunched"))?;
    let output = relaunch(h, stub, "1")?;
    h.write(&mut stream, "exit\n")?;
    h.wait(|s| !session_present(s, id(&stubborn)), 5)?;
    ensure!(
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("did not stop"),
        "Relaunch cleanup falsely succeeded for a refusing process"
    );
    ensure!(
        h.state()?
            .get("generation")
            .is_some_and(|current| current == &generation),
        "Timed-out relaunch replaced the daemon"
    );
    let marker = wait_relaunch_marker(&h.root, Duration::from_secs(5))?;
    ensure!(
        marker.contains(&format!("data={}\n", h.root.display()))
            && marker.contains(&format!("runtime={}\n", h.root.join("run").display()))
            && marker.contains("session=\n"),
        "Timed-out relaunch missed isolated env: {marker:?}"
    );
    Ok(())
}

pub(super) fn idle_shutdown_race() -> Result<()> {
    use std::sync::{Arc, Barrier};
    use terminator_core::{Paths, Request, Response, rpc};
    for _ in 0..8 {
        let h = Harness::new()?;
        h.setup()?;
        let project = h.project("idle-shutdown-race")?;
        let barrier = Arc::new(Barrier::new(2));
        let first = Arc::clone(&barrier);
        let paths = Paths::at(h.root.clone());
        let project_id = id(&project).to_owned();
        let creator = thread::spawn(move || {
            first.wait();
            rpc(
                &paths,
                Request::Create {
                    project: project_id,
                    cwd: None,
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
            )
        });
        barrier.wait();
        let shutdown = rpc(&Paths::at(h.root.clone()), Request::ShutdownIfIdle);
        let creation = creator.join().unwrap();
        match creation {
            Ok(Response::Created(session)) => {
                ensure!(
                    shutdown.is_err(),
                    "Idle shutdown acknowledged despite a successful creation"
                );
                let state = h.state()?;
                ensure!(
                    state.get("generation").and_then(Value::as_str)
                        == Some(session.generation.as_str()),
                    "Daemon changed under a live session"
                );
                h.rpc(json!({"Stop":{"session":session.id}}))?;
            }
            Err(_) => ensure!(
                matches!(shutdown, Ok(Response::Ok)),
                "Neither concurrent operation succeeded"
            ),
            _ => anyhow::bail!("Unexpected creation reply"),
        }
    }
    println!("Idle shutdown serialized against concurrent session creation (8 runs)");
    Ok(())
}
