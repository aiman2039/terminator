use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::PathBuf;

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
                crate::console::Console::err(format_args!(
                    "generation {} log: {}",
                    owner.id,
                    fs::read_to_string(owner.data.join("daemon.log")).unwrap_or_default()
                ));
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
    let mut cmd = Command::new(find_executable("nvim").expect("Install Neovim for this fixture"));
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

/// Polls `done` every `step` until it holds; fails with `message` after `limit`.
fn wait_until(limit: Duration, step: Duration, message: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now().checked_add(limit).unwrap();
    while !done() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(step);
    }
}

fn session_in<'a>(state: &'a State, id: &str) -> &'a Session {
    state.sessions.iter().find(|s| s.id == id).unwrap()
}

/// Disposable data dir plus an installation built from the workspace binaries.
struct Fixture {
    dir: tempfile::TempDir,
    paths: Paths,
    install: PathBuf,
    built: PathBuf,
    exe: PathBuf,
    _cleanup: Cleanup,
}

/// Generation A's shell (running a background job) and unsaved editor.
struct GenerationA {
    project: String,
    shell: Session,
    editor: Session,
    file: PathBuf,
    endpoint: Paths,
    job: i32,
}

impl Fixture {
    fn new() -> Self {
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
        let cleanup = Cleanup(paths.clone());
        Self {
            dir,
            paths,
            install,
            built,
            exe,
            _cleanup: cleanup,
        }
    }

    fn start_generation_a(&self) -> GenerationA {
        let paths = &self.paths;
        activate_candidate(paths, &self.exe, true).unwrap();
        rpc(
            paths,
            Request::AddProject {
                path: self.dir.path().into(),
            },
        )
        .unwrap();
        let initial = state(paths);
        let project = initial.projects[0].id.clone();
        let settings = Settings {
            shell: "/bin/sh".into(),
            os_events: std::collections::BTreeSet::default(),
            ..Default::default()
        };
        rpc(paths, Request::Settings(settings)).unwrap();
        let shell = create(paths, &project, None);
        let file = self.dir.path().join("buffer.txt");
        fs::write(&file, "saved\n").unwrap();
        let editor = create(paths, &project, Some(file.clone()));
        let endpoint = generations::owner_for(
            paths,
            &Request::History {
                session: editor.id.clone(),
            },
        )
        .unwrap();
        wait_until(
            Duration::from_secs(5),
            Duration::from_millis(50),
            "editor did not start",
            || endpoint.editor_socket(&editor.id).exists(),
        );
        nvim(paths, &editor, "setline(1, 'UNSAVED-GENERATION-A')");
        let job = self.start_background_job(&shell);
        GenerationA {
            project,
            shell,
            editor,
            file,
            endpoint,
            job,
        }
    }

    fn start_background_job(&self, shell: &Session) -> i32 {
        let job_file = self.dir.path().join("job.pid");
        input(
            &self.paths,
            shell,
            format!(
                "sleep 120 & echo $! > {}; wait\r",
                quote(&job_file.to_string_lossy())
            )
            .as_bytes(),
        );
        wait_until(
            Duration::from_secs(5),
            Duration::from_millis(50),
            "background job did not start",
            || job_file.exists(),
        );
        fs::read_to_string(&job_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    /// Neither an exec failure nor exit before storage initialization may
    /// leave a Prepared row that later prevents all-owner shutdown.
    fn rejects_unstartable_candidates(&self, current: &str) {
        let paths = &self.paths;
        for (name, binary) in [
            ("invalid-exec", b"invalid executable format\n".as_slice()),
            ("early-exit", b"#!/bin/sh\nexit 7\n".as_slice()),
        ] {
            let invalid = self.dir.path().join(name);
            fs::create_dir(&invalid).unwrap();
            fs::write(invalid.join("terminator-daemon"), binary).unwrap();
            fs::set_permissions(
                invalid.join("terminator-daemon"),
                fs::Permissions::from_mode(0o500),
            )
            .unwrap();
            copy_executable(
                &self.built.join("terminator-hook"),
                &invalid.join("terminator-hook"),
            )
            .unwrap();
            let before = Catalog::open(paths).unwrap().generations().unwrap().len();
            let directories = fs::read_dir(paths.data.join("generations"))
                .unwrap()
                .count();
            assert!(activate_candidate(paths, &invalid.join("terminator"), true).is_err());
            assert_eq!(
                Catalog::open(paths).unwrap().generations().unwrap().len(),
                before
            );
            assert_eq!(
                fs::read_dir(paths.data.join("generations"))
                    .unwrap()
                    .count(),
                directories
            );
            assert_eq!(state(paths).generation, current);
        }
    }

    /// A candidate with a broken helper must leave the current owner selected.
    fn rejects_broken_helper(&self, current: &str) {
        let broken = self.dir.path().join("broken-install");
        fs::create_dir(&broken).unwrap();
        copy_executable(
            &self.built.join("terminator-daemon"),
            &broken.join("terminator-daemon"),
        )
        .unwrap();
        fs::write(broken.join("terminator-hook"), b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(
            broken.join("terminator-hook"),
            fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        assert!(activate_candidate(&self.paths, &broken.join("terminator"), true).is_err());
        assert_eq!(state(&self.paths).generation, current);
    }
}

fn assert_sessions_intact(paths: &Paths, originals: &[&Session]) {
    let inventory = state(paths);
    for original in originals {
        let actual = session_in(&inventory, &original.id);
        assert_eq!(actual.pid, original.pid);
        assert_eq!(actual.generation, original.generation);
        assert!(actual.lifecycle.live());
        assert!(matches!(
            rpc(
                paths,
                Request::Screen {
                    session: original.id.clone()
                }
            )
            .unwrap(),
            Response::Text(_)
        ));
    }
}

/// The old owner rejects new sessions but still routes notices, saves,
/// closes and input for its own sessions.
fn assert_old_owner_routing(paths: &Paths, a: &GenerationA) {
    let before = state(&a.endpoint).sessions.len();
    assert!(
        rpc(
            &a.endpoint,
            Request::Create {
                project: a.project.clone(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false
            }
        )
        .is_err()
    );
    assert_eq!(state(&a.endpoint).sessions.len(), before);
    rpc(
        paths,
        Request::TerminalNotify {
            session: a.shell.id.clone(),
            title: "old owner".into(),
            body: "routed".into(),
        },
    )
    .unwrap();
    assert!(
        state(paths)
            .terminal_notices
            .iter()
            .any(|n| n.session_id == a.shell.id)
    );
    rpc(
        paths,
        Request::EditorSave {
            session: a.editor.id.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&a.file).unwrap(),
        "UNSAVED-GENERATION-A\n"
    );
    crate::editor_close::close(
        paths,
        std::slice::from_ref(&a.editor.id),
        crate::editor_close::Mode::Save,
        Duration::from_secs(2),
    )
    .unwrap();
    signals::signal_process(u32::try_from(a.job).unwrap(), signals::ProcSignal::Hangup).unwrap();
    input(paths, &a.shell, b"printf 'STILL-INTERACTIVE-A\\n'\r");
    let Response::Text(history) = rpc(
        paths,
        Request::History {
            session: a.shell.id.clone(),
        },
    )
    .unwrap() else {
        panic!("history");
    };
    assert!(history.contains("STILL-INTERACTIVE-A"));
}

/// Unavailability is not death; only an exited owner loses its live records.
fn assert_only_exited_owner_is_reconciled(paths: &Paths, b: &Session) {
    let b_owner = Catalog::open(paths)
        .unwrap()
        .generations()
        .unwrap()
        .into_iter()
        .find(|g| g.id == b.generation)
        .unwrap();
    let b_pid = u32::try_from(b_owner.pid.unwrap().cast_signed()).unwrap();
    assert!(signals::signal_process(b_pid, signals::ProcSignal::Stop).is_ok());
    let unavailable = state(paths);
    assert!(
        unavailable
            .generations
            .iter()
            .find(|g| g.owner.id == b.generation)
            .unwrap()
            .error
            .is_some()
    );
    assert!(session_in(&unavailable, &b.id).lifecycle.live());
    assert!(signals::signal_process(b_pid, signals::ProcSignal::Cont).is_ok());
    assert!(signals::signal_process(b_pid, signals::ProcSignal::Term).is_ok());
    wait_until(
        Duration::from_secs(8),
        Duration::from_millis(100),
        "Only the crashed owner should be reconciled",
        || !session_in(&state(paths), &b.id).lifecycle.live(),
    );
    assert_eq!(
        session_in(&state(paths), &b.id).lifecycle,
        Lifecycle::Interrupted
    );
}

fn assert_draining_owners_retire(paths: &Paths, a: &Session, b: &Session) {
    rpc(
        paths,
        Request::Stop {
            session: a.id.clone(),
        },
    )
    .unwrap();
    wait_until(
        Duration::from_secs(8),
        Duration::from_millis(100),
        "draining owners did not retire",
        || {
            Catalog::open(paths)
                .unwrap()
                .generations()
                .unwrap()
                .iter()
                .filter(|g| g.id == a.generation || g.id == b.generation)
                .all(|g| g.status == Status::Retired)
        },
    );
    assert!(
        rpc(
            paths,
            Request::History {
                session: a.id.clone()
            }
        )
        .is_ok(),
        "retired history must remain readable"
    );
}

/// Build the workspace binaries first. This test starts only disposable owners.
#[test]
#[ignore = "requires built daemon/helper binaries and Neovim; real PTYs"]
fn three_generations_preserve_shell_job_editor_and_owner_routing() {
    let fixture = Fixture::new();
    let paths = &fixture.paths;
    let a = fixture.start_generation_a();
    activate_candidate(paths, &fixture.exe, true).unwrap();
    let b = create(paths, &a.project, None);
    activate_candidate(paths, &fixture.exe, true).unwrap();
    let c = create(paths, &a.project, None);
    assert_ne!(a.shell.generation, b.generation);
    assert_ne!(b.generation, c.generation);
    assert_eq!(state(paths).generation, c.generation);
    fixture.rejects_unstartable_candidates(&c.generation);
    fixture.rejects_broken_helper(&c.generation);
    // Replacing/removing the installation cannot remove an owner's helper.
    fs::remove_dir_all(&fixture.install).unwrap();
    assert_sessions_intact(paths, &[&a.shell, &b, &c, &a.editor]);
    assert!(
        signals::process_alive(u32::try_from(a.job).unwrap()),
        "running job was lost"
    );
    assert_eq!(nvim(paths, &a.editor, "getline(1)"), "UNSAVED-GENERATION-A");
    assert_eq!(nvim(paths, &a.editor, "&modified"), "1");
    assert_eq!(fs::read_to_string(&a.file).unwrap(), "saved\n");
    assert_old_owner_routing(paths, &a);
    assert_only_exited_owner_is_reconciled(paths, &b);
    assert_draining_owners_retire(paths, &a.shell, &b);
    assert_eq!(session_in(&state(paths), &c.id).pid, c.pid);
}
