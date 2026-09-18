//! Stage and verify a candidate before atomically admitting new sessions.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
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
                let _ = self.discard_unstarted(self.child.as_ref().map(|c| c.id()));
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
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o500)
        .open(target)?;
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
        right.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
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
    if let Some(active) = catalog.active()? {
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
                &owner.data.join("bin/terminator-daemon"),
                &executable.with_file_name("terminator-daemon"),
            )?
            && same_file_contents(
                &owner.data.join("bin/terminator-hook"),
                &executable.with_file_name("terminator-hook"),
            )?
        {
            let _guard = generations::coordinate(paths)?;
            Catalog::open(paths)?.restore_serving()?;
            return Ok(());
        }
    }
    let generation = id();
    let data = paths.data.join("generations").join(&generation);
    let runtime = paths.runtime.join(&generation[..8]);
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
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700))?;
    for name in ["terminator-daemon", "terminator-hook"] {
        copy_executable(&executable.with_file_name(name), &bin.join(name))?;
    }
    // The unique staged directory identifies immutable complete executable copies.
    let owner = Generation {
        id: generation.clone(),
        data,
        runtime,
        version: env!("CARGO_PKG_VERSION").into(),
        build: generations::build_identity(&bin)?,
        protocol: PROTOCOL_VERSION,
        catalog: generations::CATALOG_VERSION,
        status: Status::Prepared,
        pid: None,
    };
    {
        let _guard = generations::coordinate(paths)?;
        catalog.register(&owner)?;
    }
    let mut pending = Candidate {
        child: None,
        paths: candidate.clone(),
        root: paths.clone(),
        generation: generation.clone(),
        activated: false,
    };
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(candidate.data.join("daemon.log"))?;
    let mut command = Command::new(bin.join("terminator-daemon"));
    command
        .env("TERMINATOR_DATA_DIR", &candidate.data)
        .env("TERMINATOR_RUNTIME_DIR", &candidate.runtime)
        .env("TERMINATOR_CATALOG_DATA", &paths.data)
        .env("TERMINATOR_CATALOG_RUNTIME", &paths.runtime)
        .env("TERMINATOR_GENERATION", &generation)
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
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    let pid = child.id();
    pending.child = Some(child);
    {
        let _guard = generations::coordinate(paths)?;
        catalog.set_pid(&generation, pid)?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Response::State(state)) = rpc(&candidate, Request::Snapshot) {
            ensure!(
                state.generation == generation
                    && state.daemon_build.as_ref() == Some(&owner.build)
                    && state.daemon_catalog_version == Some(generations::CATALOG_VERSION)
                    && state.daemon_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
                    && state
                        .capabilities
                        .iter()
                        .any(|c| c == generations::CAPABILITY)
                    && state.attachment_helper_available == Some(true)
                    && state.daemon_executable.as_ref() == Some(&bin.join("terminator-daemon")),
                "Candidate identity or helper verification failed"
            );
            let helper = state
                .attachment_helper_executable
                .as_ref()
                .context("Candidate has no helper")?;
            ensure!(
                same_file_contents(helper, &bin.join("terminator-hook"))?,
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
            let _guard = generations::coordinate(paths)?;
            catalog.activate(&generation)?;
            pending.activated = true;
            return Ok(());
        }
        ensure!(
            pending.child.as_mut().unwrap().try_wait()?.is_none(),
            "Candidate exited; previous service preserved. Inspect {}",
            paths
                .data
                .join(format!("failed-candidate-{generation}.log"))
                .display()
        );
        ensure!(
            Instant::now() < deadline,
            "Candidate verification timed out; previous service preserved"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    struct Cleanup(Paths);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let Ok(catalog) = Catalog::open(&self.0) else {
                return;
            };
            let Ok(owners) = catalog.generations() else {
                return;
            };
            for owner in owners {
                if std::thread::panicking() {
                    eprintln!(
                        "generation {} log: {}",
                        owner.id,
                        fs::read_to_string(owner.data.join("daemon.log")).unwrap_or_default()
                    );
                }
                if let Ok(Response::State(state)) = rpc(&owner.paths(), Request::Snapshot) {
                    for session in state.sessions.iter().filter(|s| s.lifecycle.live()) {
                        let _ = rpc(
                            &owner.paths(),
                            Request::Stop {
                                session: session.id.clone(),
                            },
                        );
                    }
                    for _ in 0..40 {
                        if rpc(&owner.paths(), Request::ShutdownIfIdle).is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        }
    }
    fn state(paths: &Paths) -> State {
        let Response::State(state) = rpc(paths, Request::Snapshot).unwrap() else {
            panic!("snapshot");
        };
        *state
    }
    fn create(paths: &Paths, project: &str, file: Option<std::path::PathBuf>) -> Session {
        let Response::Created(session) = rpc(
            paths,
            Request::Create {
                project: project.into(),
                cwd: None,
                editor: file.is_some(),
                file,
                line: None,
                column: None,
            },
        )
        .unwrap() else {
            panic!("create");
        };
        session
    }
    fn input(paths: &Paths, session: &Session, data: &[u8]) {
        let mut stream = connect(
            paths,
            Request::Attach {
                session: session.id.clone(),
                rows: 24,
                cols: 80,
            },
            None,
        )
        .unwrap();
        read_frame::<Response>(&mut stream)
            .unwrap()
            .checked()
            .unwrap();
        write_frame(
            &mut stream,
            &Request::Input {
                data: STANDARD.encode(data),
            },
        )
        .unwrap();
        // Keep the bridge alive until the input reader has admitted the bytes.
        thread::sleep(Duration::from_millis(100));
    }
    fn nvim(paths: &Paths, session: &Session, expression: &str) -> String {
        let owner = generations::owner_for(
            paths,
            &Request::History {
                session: session.id.clone(),
            },
        )
        .unwrap();
        let mut cmd =
            Command::new(find_executable("nvim").expect("Install Neovim for this fixture"));
        cmd.arg("--server")
            .arg(owner.editor_socket(&session.id))
            .args(["--remote-expr", expression]);
        let output = bounded_output(cmd, Duration::from_secs(3)).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().into()
    }

    /// Build the workspace binaries first. This test starts only disposable owners.
    #[test]
    #[ignore = "requires built daemon/helper binaries and Neovim; real PTYs"]
    fn three_generations_preserve_shell_job_editor_and_owner_routing() {
        let dir = tempfile::Builder::new()
            .prefix("up-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().join("data"));
        paths.init().unwrap();
        generations::migrate_idle(&paths).unwrap();
        let install = dir.path().join("install");
        fs::create_dir(&install).unwrap();
        let built = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        for name in ["terminator-daemon", "terminator-hook"] {
            copy_executable(&built.join(name), &install.join(name)).unwrap();
        }
        let exe = install.join("terminator");
        let _cleanup = Cleanup(paths.clone());
        activate_candidate(&paths, &exe, true).unwrap();
        rpc(
            &paths,
            Request::AddProject {
                path: dir.path().into(),
            },
        )
        .unwrap();
        let initial = state(&paths);
        let project = initial.projects[0].id.clone();
        let settings = Settings {
            shell: "/bin/sh".into(),
            os_events: Default::default(),
            ..Default::default()
        };
        rpc(&paths, Request::Settings(settings)).unwrap();
        let a = create(&paths, &project, None);
        let file = dir.path().join("buffer.txt");
        fs::write(&file, "saved\n").unwrap();
        let editor = create(&paths, &project, Some(file.clone()));
        let endpoint = generations::owner_for(
            &paths,
            &Request::History {
                session: editor.id.clone(),
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !endpoint.editor_socket(&editor.id).exists() {
            assert!(Instant::now() < deadline, "editor did not start");
            thread::sleep(Duration::from_millis(50));
        }
        nvim(&paths, &editor, "setline(1, 'UNSAVED-GENERATION-A')");
        let job_file = dir.path().join("job.pid");
        input(
            &paths,
            &a,
            format!(
                "sleep 120 & echo $! > {}; wait\r",
                quote(&job_file.to_string_lossy())
            )
            .as_bytes(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !job_file.exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(50));
        }
        let job: i32 = fs::read_to_string(&job_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        activate_candidate(&paths, &exe, true).unwrap();
        let b = create(&paths, &project, None);
        activate_candidate(&paths, &exe, true).unwrap();
        let c = create(&paths, &project, None);
        assert_ne!(a.generation, b.generation);
        assert_ne!(b.generation, c.generation);
        assert_eq!(state(&paths).generation, c.generation);
        // Neither an exec failure nor exit before storage initialization may
        // leave a Prepared row that later prevents all-owner shutdown.
        for (name, binary) in [
            ("invalid-exec", b"invalid executable format\n".as_slice()),
            ("early-exit", b"#!/bin/sh\nexit 7\n".as_slice()),
        ] {
            let invalid = dir.path().join(name);
            fs::create_dir(&invalid).unwrap();
            fs::write(invalid.join("terminator-daemon"), binary).unwrap();
            fs::set_permissions(
                invalid.join("terminator-daemon"),
                fs::Permissions::from_mode(0o500),
            )
            .unwrap();
            copy_executable(
                &built.join("terminator-hook"),
                &invalid.join("terminator-hook"),
            )
            .unwrap();
            let before = Catalog::open(&paths).unwrap().generations().unwrap().len();
            let directories = fs::read_dir(paths.data.join("generations"))
                .unwrap()
                .count();
            assert!(activate_candidate(&paths, &invalid.join("terminator"), true).is_err());
            assert_eq!(
                Catalog::open(&paths).unwrap().generations().unwrap().len(),
                before
            );
            assert_eq!(
                fs::read_dir(paths.data.join("generations"))
                    .unwrap()
                    .count(),
                directories
            );
            assert_eq!(state(&paths).generation, c.generation);
        }
        // A candidate with a broken helper must leave C selected and all sessions intact.
        let broken = dir.path().join("broken-install");
        fs::create_dir(&broken).unwrap();
        copy_executable(
            &built.join("terminator-daemon"),
            &broken.join("terminator-daemon"),
        )
        .unwrap();
        fs::write(broken.join("terminator-hook"), b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(
            broken.join("terminator-hook"),
            fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        assert!(activate_candidate(&paths, &broken.join("terminator"), true).is_err());
        assert_eq!(state(&paths).generation, c.generation);
        // Replacing/removing the installation cannot remove an owner's helper.
        fs::remove_dir_all(&install).unwrap();
        let inventory = state(&paths);
        for original in [&a, &b, &c, &editor] {
            let actual = inventory
                .sessions
                .iter()
                .find(|s| s.id == original.id)
                .unwrap();
            assert_eq!(actual.pid, original.pid);
            assert_eq!(actual.generation, original.generation);
            assert!(actual.lifecycle.live());
            assert!(matches!(
                rpc(
                    &paths,
                    Request::Screen {
                        session: original.id.clone()
                    }
                )
                .unwrap(),
                Response::Text(_)
            ));
        }
        assert_eq!(unsafe { libc::kill(job, 0) }, 0, "running job was lost");
        assert_eq!(nvim(&paths, &editor, "getline(1)"), "UNSAVED-GENERATION-A");
        assert_eq!(nvim(&paths, &editor, "&modified"), "1");
        assert_eq!(fs::read_to_string(&file).unwrap(), "saved\n");
        // A pre-spawn rejection leaves the old generation's inventory unchanged.
        let before = state(&endpoint).sessions.len();
        assert!(
            rpc(
                &endpoint,
                Request::Create {
                    project: project.clone(),
                    cwd: None,
                    file: None,
                    line: None,
                    column: None,
                    editor: false
                }
            )
            .is_err()
        );
        assert_eq!(state(&endpoint).sessions.len(), before);
        rpc(
            &paths,
            Request::TerminalNotify {
                session: a.id.clone(),
                title: "old owner".into(),
                body: "routed".into(),
            },
        )
        .unwrap();
        assert!(
            state(&paths)
                .terminal_notices
                .iter()
                .any(|n| n.session_id == a.id)
        );
        rpc(
            &paths,
            Request::EditorSave {
                session: editor.id.clone(),
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "UNSAVED-GENERATION-A\n");
        crate::editor_close::close(
            &paths,
            std::slice::from_ref(&editor.id),
            crate::editor_close::Mode::Save,
            Duration::from_secs(2),
        )
        .unwrap();
        unsafe {
            libc::kill(job, libc::SIGHUP);
        }
        input(&paths, &a, b"printf 'STILL-INTERACTIVE-A\\n'\r");
        let Response::Text(history) = rpc(
            &paths,
            Request::History {
                session: a.id.clone(),
            },
        )
        .unwrap() else {
            panic!("history");
        };
        assert!(history.contains("STILL-INTERACTIVE-A"));
        // Unavailability is not death; only an exited owner loses its live records.
        let b_owner = Catalog::open(&paths)
            .unwrap()
            .generations()
            .unwrap()
            .into_iter()
            .find(|g| g.id == b.generation)
            .unwrap();
        let b_pid = b_owner.pid.unwrap() as i32;
        assert_eq!(unsafe { libc::kill(b_pid, libc::SIGSTOP) }, 0);
        let unavailable = state(&paths);
        assert!(
            unavailable
                .generations
                .iter()
                .find(|g| g.owner.id == b.generation)
                .unwrap()
                .error
                .is_some()
        );
        assert!(
            unavailable
                .sessions
                .iter()
                .find(|s| s.id == b.id)
                .unwrap()
                .lifecycle
                .live()
        );
        assert_eq!(unsafe { libc::kill(b_pid, libc::SIGCONT) }, 0);
        assert_eq!(unsafe { libc::kill(b_pid, libc::SIGTERM) }, 0);
        let deadline = Instant::now() + Duration::from_secs(8);
        while state(&paths)
            .sessions
            .iter()
            .find(|s| s.id == b.id)
            .unwrap()
            .lifecycle
            .live()
        {
            assert!(
                Instant::now() < deadline,
                "Only the crashed owner should be reconciled"
            );
            thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            state(&paths)
                .sessions
                .iter()
                .find(|s| s.id == b.id)
                .unwrap()
                .lifecycle,
            Lifecycle::Interrupted
        );
        rpc(
            &paths,
            Request::Stop {
                session: a.id.clone(),
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let owners = Catalog::open(&paths).unwrap().generations().unwrap();
            if owners
                .iter()
                .filter(|g| g.id == a.generation || g.id == b.generation)
                .all(|g| g.status == Status::Retired)
            {
                break;
            }
            assert!(Instant::now() < deadline, "draining owners did not retire");
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            rpc(&paths, Request::History { session: a.id }).is_ok(),
            "retired history must remain readable"
        );
        assert_eq!(
            state(&paths)
                .sessions
                .iter()
                .find(|s| s.id == c.id)
                .unwrap()
                .pid,
            c.pid
        );
    }
}
