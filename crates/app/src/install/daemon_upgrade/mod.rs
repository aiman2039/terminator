//! Stage and verify a candidate before atomically admitting new sessions.
#[cfg(test)]
#[cfg(unix)]
mod tests;

use anyhow::{Context, Result, ensure};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{
    generations::{self, Catalog, Generation, Status},
    *,
};

struct Candidate {
    child: Option<std::process::Child>,
    paths: Paths,
    root: Paths,
    generation: String,
    activated: bool,
}
impl Candidate {
    fn discard_unstarted(&self, pid: Option<u32>) -> Result<()> {
        use fs2::FileExt;
        let _guard = generations::coordinate(&self.root)?;
        // Even after a Child exits, retain any directory whose owner lock is held.
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.paths.runtime.join("daemon.lock"))?;
        lock.try_lock_exclusive()?;
        if Catalog::open(&self.root)?.discard_prepared(&self.generation, pid)? {
            let log = self.paths.data.join("daemon.log");
            if log.is_file() {
                fs::copy(
                    &log,
                    self.root
                        .data
                        .join(format!("failed-candidate-{}.log", self.generation)),
                )?;
            }
            fs::remove_dir_all(&self.paths.data)?;
            fs::remove_dir_all(&self.paths.runtime)?;
        }
        Ok(())
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if !self.activated {
            let exited = self
                .child
                .as_mut()
                .is_some_and(|child| child.try_wait().ok().flatten().is_some());
            if self.child.is_none() || exited {
                let _ = self.discard_unstarted(self.child.as_ref().map(std::process::Child::id));
            } else {
                // A running or uncertain candidate may only acknowledge idle shutdown.
                let _ = rpc(&self.paths, Request::ShutdownIfIdle);
            }
        }
        if let Some(mut child) = self.child.take() {
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn copy_executable(source: &Path, target: &Path) -> Result<()> {
    ensure!(
        executable_available(source),
        "Missing executable: {}",
        source.display()
    );
    let mut source = fs::File::open(source)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o500);
    let mut output = options.open(target)?;
    std::io::copy(&mut source, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

pub fn same_file_contents(left: &Path, right: &Path) -> Result<bool> {
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    if left.metadata()?.len() != right.metadata()?.len() {
        return Ok(false);
    }
    let mut a = [0; 65536];
    let mut b = [0; 65536];
    loop {
        let n = left.read(&mut a)?;
        if n == 0 {
            return Ok(true);
        }
        let Some(b_buf) = b.get_mut(..n) else {
            return Ok(false);
        };
        right.read_exact(b_buf)?;
        if a.get(..n) != b.get(..n) {
            return Ok(false);
        }
    }
}

pub fn activate(paths: &Paths, executable: &Path) -> Result<()> {
    activate_candidate(
        paths,
        executable,
        cfg!(feature = "test-support")
            && std::env::var_os("TERMINATOR_TEST_NEW_GENERATION").is_some(),
    )
}

fn activate_candidate(paths: &Paths, executable: &Path, force: bool) -> Result<()> {
    // Serialize installers separately: candidates must answer IPC while activation
    // owns the coordination lock, and existing sessions remain fully interactive.
    use fs2::FileExt;
    let install = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join("activation.lock"))?;
    install.lock_exclusive()?;
    let mut catalog = Catalog::open(paths)?;
    for owner in catalog
        .generations()?
        .iter()
        .filter(|g| g.status != Status::Retired)
    {
        let _ = generations::recover_exited(paths, owner);
    }
    if active_matches_bundle(&catalog, executable, force)? {
        let _guard = generations::coordinate(paths)?;
        Catalog::open(paths)?.restore_serving()?;
        return Ok(());
    }
    let staged = stage_candidate(paths, executable)?;
    {
        let _guard = generations::coordinate(paths)?;
        catalog.register(&staged.owner)?;
    }
    let mut pending = Candidate {
        child: None,
        paths: staged.paths.clone(),
        root: paths.clone(),
        generation: staged.owner.id.clone(),
        activated: false,
    };
    let child = spawn_candidate(paths, &staged, executable)?;
    let pid = child.id();
    pending.child = Some(child);
    {
        let _guard = generations::coordinate(paths)?;
        catalog.set_pid(&staged.owner.id, pid)?;
    }
    let Some(deadline) = Instant::now().checked_add(Duration::from_secs(10)) else {
        anyhow::bail!("Candidate verification clock overflow; previous service preserved");
    };
    loop {
        if let Ok(Response::State(state)) = rpc(&staged.paths, Request::Snapshot) {
            verify_candidate(&state, &staged)?;
            let _guard = generations::coordinate(paths)?;
            catalog.activate(&staged.owner.id)?;
            pending.activated = true;
            return Ok(());
        }
        ensure!(
            pending
                .child
                .as_mut()
                .context("candidate process handle missing")?
                .try_wait()?
                .is_none(),
            "Candidate exited; previous service preserved. Inspect {}",
            paths
                .data
                .join(format!("failed-candidate-{}.log", staged.owner.id))
                .display()
        );
        ensure!(
            Instant::now() < deadline,
            "Candidate verification timed out; previous service preserved"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// The active owner already runs this exact bundle with a working helper,
/// so it only needs to resume serving. A newer active owner is refused.
fn active_matches_bundle(catalog: &Catalog, executable: &Path, force: bool) -> Result<bool> {
    let Some(active) = catalog.active()? else {
        return Ok(false);
    };
    let owner = catalog
        .generations()?
        .into_iter()
        .find(|g| g.id == active)
        .context("Unknown active owner")?;
    let running = semver::Version::parse(&owner.version)?;
    let bundled = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
    ensure!(
        running <= bundled,
        "A newer session service is already active; refusing downgrade"
    );
    if !force
        && running == bundled
        && let Ok(Response::State(state)) = rpc(&owner.paths(), Request::Snapshot)
        && state.attachment_helper_available == Some(true)
        && same_file_contents(
            &owner.data.join("bin").join(exe_name("terminator-daemon")),
            &sibling_exe(executable, "terminator-daemon"),
        )?
        && same_file_contents(
            &owner.data.join("bin").join(exe_name("terminator-hook")),
            &sibling_exe(executable, "terminator-hook"),
        )?
    {
        return Ok(true);
    }
    Ok(false)
}

/// A candidate owner staged on disk with private executable copies.
struct Staged {
    paths: Paths,
    bin: std::path::PathBuf,
    owner: Generation,
}

fn stage_candidate(paths: &Paths, executable: &Path) -> Result<Staged> {
    let generation = id();
    let data = paths.data.join("generations").join(&generation);
    let runtime = paths.runtime.join(
        generation
            .get(..8)
            .context("generation id is shorter than 8 bytes")?,
    );
    let candidate = Paths {
        data: data.clone(),
        runtime: runtime.clone(),
    };
    candidate.init()?;
    atomic_write(
        &candidate.data.join("workspace.json"),
        &serde_json::to_vec(paths)?,
    )?;
    let bin = data.join("bin");
    fs::create_dir(&bin)?;
    #[cfg(unix)]
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700))?;
    for name in bundled_exes() {
        copy_executable(&sibling_exe(executable, &name), &bin.join(&name))?;
    }
    // The unique staged directory identifies immutable complete executable copies.
    let owner = Generation {
        id: generation,
        data,
        runtime,
        version: env!("CARGO_PKG_VERSION").into(),
        build: generations::build_identity(&bin)?,
        protocol: PROTOCOL_VERSION,
        catalog: generations::CATALOG_VERSION,
        status: Status::Prepared,
        pid: None,
    };
    Ok(Staged {
        paths: candidate,
        bin,
        owner,
    })
}

fn spawn_candidate(
    paths: &Paths,
    staged: &Staged,
    executable: &Path,
) -> Result<std::process::Child> {
    let candidate = &staged.paths;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(candidate.data.join("daemon.log"))?;
    let mut command = Command::new(staged.bin.join(exe_name("terminator-daemon")));
    command
        .env("TERMINATOR_DATA_DIR", &candidate.data)
        .env("TERMINATOR_RUNTIME_DIR", &candidate.runtime)
        .env("TERMINATOR_CATALOG_DATA", &paths.data)
        .env("TERMINATOR_CATALOG_RUNTIME", &paths.runtime)
        .env("TERMINATOR_GENERATION", &staged.owner.id)
        .env("TERMINATOR_GUI_EXECUTABLE", executable)
        .env_remove("TERMINATOR_SESSION_ID")
        .env_remove("TERMINATOR_SESSION_TOKEN")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(test)]
    command
        .env("XDG_CONFIG_HOME", paths.data.join("fixture-config"))
        .env("XDG_STATE_HOME", paths.data.join("fixture-state"))
        .env("TERMINATOR_NO_NOTIFICATIONS", "1");
    terminator_sys::detach_session(&mut command);
    Ok(command.spawn()?)
}

/// The running candidate must report the staged identity and run a private
/// helper that matches the staged copy and passes its health check.
fn verify_candidate(state: &State, staged: &Staged) -> Result<()> {
    let bin = &staged.bin;
    ensure!(
        state.generation == staged.owner.id
            && state.daemon_build.as_ref() == Some(&staged.owner.build)
            && state.daemon_catalog_version == Some(generations::CATALOG_VERSION)
            && state.daemon_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
            && state
                .capabilities
                .iter()
                .any(|c| c == generations::CAPABILITY)
            && state.attachment_helper_available == Some(true)
            && state.daemon_executable.as_ref() == Some(&bin.join(exe_name("terminator-daemon"))),
        "Candidate identity or helper verification failed"
    );
    let helper = state
        .attachment_helper_executable
        .as_ref()
        .context("Candidate has no helper")?;
    ensure!(
        same_file_contents(helper, &bin.join(exe_name("terminator-hook")))?,
        "Candidate private helper does not match the staged helper"
    );
    let mut health = Command::new(helper);
    health.arg("--health-check");
    let result = bounded_output(health, Duration::from_secs(10))?;
    ensure!(
        result.status.success()
            && String::from_utf8_lossy(&result.stdout).trim()
                == format!("{}:{}", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION),
        "Candidate helper health check failed"
    );
    Ok(())
}
