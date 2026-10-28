//! Explicit cleanup through daemon-owned session termination, never process-name kills.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

pub struct Options {
    pub stop_all: bool,
    pub confirmed: Option<recovery::RestartInventory>,
    pub timeout: Duration,
    pub relaunch: Option<PathBuf>,
}

fn current(paths: &Paths, generation: &str) -> Result<State> {
    let Response::State(state) = rpc(paths, Request::Snapshot)? else {
        anyhow::bail!("Could not read session inventory");
    };
    ensure!(
        state.generation == generation,
        "The daemon changed during cleanup; retry against the current service"
    );
    Ok(*state)
}

fn check_scope(state: &State, targets: &HashSet<String>) -> Result<()> {
    ensure!(
        state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .all(|s| targets.contains(&s.id)),
        "A new session started during cleanup. It was preserved; stop other clients creating sessions and retry"
    );
    Ok(())
}

fn try_lock(file: &File) -> Result<bool> {
    lock_result(file.try_lock_exclusive())
}

fn lock_result(result: std::io::Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        // Windows reports ERROR_LOCK_VIOLATION, which need not map to
        // WouldBlock. Use fs2's platform-specific contention error.
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn close_gui(paths: &Paths, timeout: Duration) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options.open(paths.runtime.join("ui.lock"))?;
    if !try_lock(&lock)? {
        ui_control::rpc(paths, ui_control::Request::Window { action: "close".into() })
            .context("Could not request a normal GUI exit. If Terminator is hidden or minimized, restore its window, quit it completely and retry; sessions have not been stopped")?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        while !try_lock(&lock)? {
            ensure!(
                Instant::now() < deadline,
                "GUI did not finish saving and closing. Sessions have not been stopped; finish or cancel pending dialogs and retry"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
    // Keep the lock until cleanup completes, so another GUI cannot create sessions.
    Ok(lock)
}

fn refuse_managed_session(state: &State) -> Result<()> {
    ensure!(
        std::env::var_os("TERMINATOR_SESSION_ID").is_none(),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    let ancestors = crate::hook_event::HookCommand::ancestors();
    ensure!(
        !state
            .sessions
            .iter()
            .any(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| ancestors.contains(&pid))),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    Ok(())
}

fn validate_relaunch(exe: Option<&Path>) -> Result<()> {
    let Some(exe) = exe else {
        return Ok(());
    };
    ensure!(
        executable_available(exe),
        "Cannot reopen Terminator: {} is unavailable",
        exe.display()
    );
    Ok(())
}

fn prepare_relaunch_command(paths: &Paths, exe: &Path) -> Result<Command> {
    let config = terminator_core::appearance::config_path(paths)?;
    let mut command = Command::new(exe);
    command
        .env("TERMINATOR_DATA_DIR", &paths.data)
        .env("TERMINATOR_RUNTIME_DIR", &paths.runtime)
        .env(
            "TERMINATOR_CONFIG_DIR",
            config.parent().context("Missing config directory")?,
        )
        .env_remove("TERMINATOR_SESSION_ID")
        .env_remove("TERMINATOR_SESSION_TOKEN")
        .stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("TERMINATOR_TEST_ACTIONS") || name.starts_with("TERMINATOR_CAPTURE_") {
            command.env_remove(key);
        }
    }
    Ok(command)
}

fn spawn_relaunch(paths: &Paths, exe: &Path) -> Result<()> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.data.join("restart.log"))?;
    let mut command = prepare_relaunch_command(paths, exe)?;
    command.stdout(log.try_clone()?).stderr(log);
    let mut child = spawn_session_leader(command)?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn stop_session(
    paths: &Paths,
    generation: &str,
    targets: &HashSet<String>,
    sid: &str,
) -> Result<()> {
    let before = current(paths, generation)?;
    check_scope(&before, targets)?;
    if before
        .sessions
        .iter()
        .any(|s| s.id == sid && s.lifecycle.live())
        && let Err(error) = rpc(
            paths,
            Request::Stop {
                session: sid.to_owned(),
            },
        )
    {
        let after = current(paths, generation)?;
        if after
            .sessions
            .iter()
            .any(|s| s.id == sid && s.lifecycle.live())
        {
            return Err(anyhow::Error::from(error).context(format!(
                "Could not stop session {sid}; the daemon was left running"
            )));
        }
    }
    Ok(())
}

fn wait_until_idle(
    paths: &Paths,
    generation: &str,
    targets: &HashSet<String>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        let remaining = current(paths, generation)?;
        check_scope(&remaining, targets)?;
        let live = remaining
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .count();
        if live == 0 {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "{live} sessions did not stop within the timeout. The daemon was left running; close the remaining jobs and retry"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_daemon_exit(paths: &Paths, timeout: Duration) -> Result<()> {
    let lock = OpenOptions::new()
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        if !paths.socket().exists() && try_lock(&lock)? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Shutdown was acknowledged but the daemon has not exited; do not relaunch until it finishes"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn finish_cleanup(paths: &Paths, state: State, stop_all: bool, timeout: Duration) -> Result<usize> {
    let targets: HashSet<_> = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .map(|s| s.id.clone())
        .collect();
    let state = current(paths, &state.generation)?;
    check_scope(&state, &targets)?;
    ensure!(
        stop_all || targets.is_empty(),
        "A session started while the GUI was closing. It was preserved; save your work and retry with --stop-all"
    );
    if !targets.is_empty() {
        crate::output::Output::warn(format_args!(
            "Stopping {} sessions; unsaved editor buffers will be discarded.",
            targets.len()
        ));
    }
    let retiring = if generations::exists(paths) {
        generations::Catalog::open(paths)?
            .generations()?
            .into_iter()
            .filter(|g| g.status != generations::Status::Retired)
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    for sid in &targets {
        stop_session(paths, &state.generation, &targets, sid)?;
    }
    wait_until_idle(paths, &state.generation, &targets, timeout)?;
    let request = if state
        .capabilities
        .iter()
        .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY)
    {
        Request::ShutdownIfIdle
    } else {
        Request::Shutdown
    };
    ensure!(
        matches!(rpc(paths, request)?, Response::Ok),
        "Daemon shutdown was not acknowledged"
    );
    if generations::exists(paths) {
        for owner in retiring {
            let endpoint = owner.paths();
            if owner.id == state.generation {
                wait_daemon_exit(&endpoint, timeout)?;
                continue;
            }
            match rpc(&endpoint, Request::ShutdownIfIdle) {
                Ok(Response::Ok) => wait_daemon_exit(&endpoint, timeout)?,
                result => {
                    let retired = generations::Catalog::open(paths)?
                        .generations()?
                        .iter()
                        .any(|g| g.id == owner.id && g.status == generations::Status::Retired);
                    ensure!(
                        retired,
                        "Owner {} did not acknowledge shutdown: {result:?}",
                        owner.id
                    );
                    wait_daemon_exit(&endpoint, timeout)?;
                }
            }
        }
    } else {
        wait_daemon_exit(paths, timeout)?;
    }
    Ok(targets.len())
}

/// Re-enables owner creation when cleanup ends, however it ends.
struct Thaw<'a>(&'a Paths);
impl Drop for Thaw<'_> {
    fn drop(&mut self) {
        if let Ok(_guard) = generations::coordinate(self.0)
            && let Ok(catalog) = generations::Catalog::open(self.0)
        {
            let _ = catalog.freeze(false);
        }
    }
}

pub fn run(paths: &Paths, state: State, options: Options) -> Result<Value> {
    let Options {
        stop_all,
        timeout,
        relaunch,
        confirmed,
    } = options;
    preflight(
        &state,
        Preflight {
            stop_all,
            confirmed: confirmed.as_ref(),
            relaunch: relaunch.as_deref(),
        },
    )?;
    let thaw = freeze(paths, &state, confirmed.as_ref())?;
    let lock = close_gui(paths, timeout)?;
    let generation = state.generation.clone();
    let result = finish_cleanup(paths, state, stop_all, timeout);
    // Persist before reopening: the old GUI and its error receiver are already gone.
    let report_result = if relaunch.is_some() {
        write_restart_result(paths, &generation, &result)
    } else {
        Ok(())
    };
    drop(lock);
    drop(thaw);
    let relaunch_error = relaunch
        .as_ref()
        .and_then(|exe| spawn_relaunch(paths, exe).err());
    // Restore the GUI even when writing diagnostics fails.
    let result = result.and_then(|stopped| report_result.map(|()| stopped));
    match (result, relaunch_error) {
        (Ok(stopped), None) => Ok(json!({
            "shutdown": true,
            "stopped_sessions": stopped,
            "relaunched": relaunch.is_some()
        })),
        (Ok(_), Some(error)) => {
            Err(error).context("Sessions stopped but Terminator could not reopen")
        }
        (Err(error), _) => Err(error),
    }
}

/// The shutdown options checked before anything is frozen or closed.
struct Preflight<'a> {
    stop_all: bool,
    confirmed: Option<&'a recovery::RestartInventory>,
    relaunch: Option<&'a Path>,
}

fn preflight(state: &State, checks: Preflight<'_>) -> Result<()> {
    let Preflight {
        stop_all,
        confirmed,
        relaunch,
    } = checks;
    if let Some(confirmed) = confirmed {
        confirmed.validate(state)?;
    }
    refuse_managed_session(state)?;
    let live = state.sessions.iter().filter(|s| s.lifecycle.live()).count();
    ensure!(
        stop_all || live == 0,
        "{live} live sessions remain. Save your work, then use --stop-all to terminate them (unsaved editor buffers will be lost)"
    );
    validate_relaunch(relaunch)
}

/// Freeze owner creation, then confirm the frozen inventory still matches.
fn freeze<'a>(
    paths: &'a Paths,
    state: &State,
    confirmed: Option<&recovery::RestartInventory>,
) -> Result<Option<Thaw<'a>>> {
    if !generations::exists(paths) {
        return Ok(None);
    }
    let thaw = {
        let _guard = generations::coordinate(paths)?;
        generations::Catalog::open(paths)?.freeze(true)?;
        Thaw(paths)
    };
    let frozen = current(paths, &state.generation)?;
    if let Some(confirmed) = confirmed {
        confirmed.validate(&frozen)?;
    }
    let confirmed: HashSet<_> = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .map(|s| s.id.clone())
        .collect();
    check_scope(&frozen, &confirmed)?;
    ensure!(
        frozen.generations.iter().all(|g| g.error.is_none()),
        "An owner is unavailable; no sessions were stopped"
    );
    Ok(Some(thaw))
}

fn write_restart_result(paths: &Paths, generation: &str, result: &Result<usize>) -> Result<()> {
    let report = json!({
        "generation": generation,
        "error": result.as_ref().err().map(|error| format!("{error:#}")),
    });
    atomic_write(
        &paths.data.join("restart-result.json"),
        &serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
