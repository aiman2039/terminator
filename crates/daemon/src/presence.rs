//! Live agent-presence inspection. One background pass every two seconds
//! batches all owned live shell sessions against a single process inventory.
//! Observations never create lifecycle events, notifications, or resume
//! commands, and never touch `SQLite`.
use super::*;
use terminator_core::agents;

/// One target per owned live shell session, with its PTY foreground group.
fn targets(shared: &Shared) -> Vec<agents::ShellTarget> {
    let live: Vec<(String, String, u32, u64)> = relock(&shared.state)
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live() && s.kind == SessionKind::Shell)
        .filter_map(|s| {
            s.pid
                .map(|pid| (s.id.clone(), s.generation.clone(), pid, s.created))
        })
        .collect();
    #[cfg(unix)]
    let runtimes = relock(&shared.sessions);
    live.into_iter()
        .map(|(id, generation, pid, created)| {
            // ConPTY exposes no process-group leader; foreground detection
            // stays Unix-only until a Windows equivalent lands.
            #[cfg(unix)]
            let foreground_pgid = runtimes
                .get(&id)
                .and_then(|runtime| relock(runtime).master.process_group_leader())
                .and_then(|pgid| u32::try_from(pgid).ok())
                .filter(|pgid| *pgid > 1);
            #[cfg(not(unix))]
            let foreground_pgid = None;
            agents::ShellTarget {
                session_id: id,
                generation,
                pid,
                start_time: created,
                foreground_pgid,
            }
        })
        .collect()
}

/// One batched inspection pass. Runs only on the inspector thread: passes
/// never overlap and never run during GUI rendering.
fn inspect_once(shared: &Shared, system: &mut sysinfo::System) {
    let targets = targets(shared);
    if targets.is_empty() {
        let mut state = relock(&shared.state);
        if !state.presence.is_empty() {
            state.presence.clear();
            state.revision = state.revision.saturating_add(1);
        }
        return;
    }
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_exe(sysinfo::UpdateKind::Always)
            .with_cmd(sysinfo::UpdateKind::Always),
    );
    let procs = agents::snapshot_processes(system);
    let presence = agents::inspect_shells(&procs, &targets, now());
    let mut state = relock(&shared.state);
    update_presence(&mut state, presence);
    // Deliberately no persist: presence never touches SQLite.
}

/// Snapshot requests check this independently of the inspector, including before
/// conditional replies. A stalled/panicked inspector must not leave verified
/// observations behind while the IPC thread continues to answer successfully.
pub(super) fn expire_observations(state: &mut State, moment: u64) {
    let mut changed = false;
    for presence in &mut state.presence {
        if presence.outcome == agents::PresenceOutcome::Verified
            && !agents::observation_fresh(presence.observed_at, moment)
        {
            presence.outcome = agents::PresenceOutcome::Unavailable;
            changed = true;
        }
    }
    if changed {
        state.revision = state.revision.saturating_add(1);
    }
}

fn update_presence(state: &mut State, presence: Vec<agents::TerminalPresence>) {
    if presence_changed(&state.presence, &presence) {
        state.revision = state.revision.saturating_add(1);
    }
    // Hook linking still needs the latest successful inspection timestamp.
    state.presence = presence;
}

fn presence_changed(
    previous: &[agents::TerminalPresence],
    current: &[agents::TerminalPresence],
) -> bool {
    previous.len() != current.len()
        || previous.iter().zip(current).any(|(a, b)| {
            a.session_id != b.session_id
                || a.generation != b.generation
                || a.agents != b.agents
                || a.outcome != b.outcome
        })
}

/// Spawn the single inspector thread. Sleep-then-inspect keeps exactly one
/// pass per interval with no overlapping scans.
pub(super) fn start(shared: std::sync::Weak<Shared>) {
    thread::spawn(move || {
        let mut system = sysinfo::System::new();
        loop {
            thread::sleep(Duration::from_secs(agents::INSPECTION_INTERVAL_SECS));
            let Some(shared) = shared.upgrade() else {
                break;
            };
            if shared.shutdown.load(Ordering::Relaxed) {
                break;
            }
            inspect_once(&shared, &mut system);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    #[test]
    fn unchanged_inspection_keeps_revision_but_refreshes_hook_observation() {
        let mut state = State::default();
        let mut current = vec![agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: agents::PresenceOutcome::Verified,
            observed_at: 1,
        }];
        update_presence(&mut state, current.clone());
        let revision = state.revision;
        current[0].observed_at = 3;
        update_presence(&mut state, current.clone());
        assert_eq!(state.revision, revision);
        assert_eq!(state.presence[0].observed_at, 3);
        current[0].agents.push(agents::DetectedAgent {
            kind: "codex".into(),
            process: agents::ProcessIdentity {
                pid: 42,
                start_time: 1,
            },
            foreground: false,
        });
        update_presence(&mut state, current.clone());
        assert_eq!(state.revision, revision + 1);
        current[0].agents[0].foreground = true;
        update_presence(&mut state, current.clone());
        assert_eq!(state.revision, revision + 2);
        current[0].outcome = agents::PresenceOutcome::Unavailable;
        update_presence(&mut state, current);
        assert_eq!(state.revision, revision + 3);
        update_presence(&mut state, vec![]);
        assert_eq!(state.revision, revision + 4);
    }

    #[test]
    fn stalled_inspection_expires_once_and_recovers_on_next_scan() {
        let mut state = State::default();
        let mut observation = agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![agents::DetectedAgent {
                kind: "codex".into(),
                process: agents::ProcessIdentity {
                    pid: 42,
                    start_time: 1,
                },
                foreground: true,
            }],
            outcome: agents::PresenceOutcome::Verified,
            observed_at: 100,
        };
        update_presence(&mut state, vec![observation.clone()]);
        let verified_hint = state.snapshot_hint();
        let revision = state.revision;
        expire_observations(&mut state, 105);
        assert_eq!(state.snapshot_hint(), verified_hint);
        expire_observations(&mut state, 106);
        assert_ne!(
            state.snapshot_hint(),
            verified_hint,
            "expiry must prevent Unchanged"
        );
        assert_eq!(state.revision, revision + 1);
        assert_eq!(
            state.presence[0].outcome,
            agents::PresenceOutcome::Unavailable
        );
        assert_eq!(state.presence[0].observed_at, 100);
        let expired_hint = state.snapshot_hint();
        expire_observations(&mut state, 200);
        assert_eq!(
            state.snapshot_hint(),
            expired_hint,
            "do not bump repeatedly while stalled"
        );
        observation.observed_at = 201;
        update_presence(&mut state, vec![observation]);
        expire_observations(&mut state, 201);
        assert_ne!(state.snapshot_hint(), expired_hint);
        assert_eq!(state.revision, revision + 2);
        assert!(agents::presence_verified(state.presence.first(), 201));
    }

    /// Renamed copies of this test binary run only
    /// [`presence_fixture_sleeper`], so fixtures verify appearance and
    /// removal without launching paid agents. (Copies of system binaries are
    /// `SIGKILLed` by macOS code-signing enforcement.)
    #[test]
    fn presence_fixture_sleeper() {
        if std::env::var_os("TERMINATOR_FIXTURE_SLEEPER").is_none() {
            return;
        }
        std::thread::sleep(Duration::from_mins(1));
    }

    /// Serializes the real-process fixture tests. Each poll does full-system
    /// exe+cmd scans; two concurrent scanners on a loaded CI runner starve
    /// each other, which flaked ubuntu-24.04. Poison-tolerant so a panicked
    /// holder cannot block the next test.
    #[cfg(unix)]
    static FIXTURE_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
        FIXTURE_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(unix)]
    fn fixture_refresh_kind() -> sysinfo::ProcessRefreshKind {
        sysinfo::ProcessRefreshKind::nothing()
            .with_exe(sysinfo::UpdateKind::Always)
            .with_cmd(sysinfo::UpdateKind::Always)
    }

    #[cfg(unix)]
    fn fixture_system() -> sysinfo::System {
        sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing().with_processes(fixture_refresh_kind()),
        )
    }

    /// Fake agent executables: renamed test-binary copies running the sleeper.
    #[cfg(unix)]
    struct Fixture {
        dir: tempfile::TempDir,
        shell: std::process::Child,
        /// Held for the fixture lifetime so real-process tests never scan
        /// the system process table concurrently.
        _serial: std::sync::MutexGuard<'static, ()>,
        system: sysinfo::System,
        shell_start_time: u64,
    }
    #[cfg(unix)]
    impl Fixture {
        fn spawn(agents: &[&str]) -> Self {
            // First: concurrent full-system scanners starve each other.
            let serial = serial_guard();
            let dir = tempfile::tempdir().unwrap();
            let current = std::env::current_exe().unwrap();
            let mut launches = Vec::new();
            for agent in agents {
                let exe = dir.path().join(agent);
                fs::copy(&current, &exe).unwrap();
                fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
                // --exact matches the full test path only; the short name
                // matches zero tests and the fixture would exit immediately.
                launches.push(format!(
                    "TERMINATOR_FIXTURE_SLEEPER=1 '{}' presence::tests::presence_fixture_sleeper --exact --nocapture &",
                    exe.display()
                ));
            }
            // Trailing foreground sleep keeps the shell alive after the
            // agents exit so removal is observed as verified-empty.
            launches.push("sleep 60".into());
            let mut command = std::process::Command::new("sh");
            command
                .arg("-c")
                .arg(launches.join(" "))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // Own process group so teardown kills shell and agents together.
            command.process_group(0);
            let shell = command.spawn().unwrap();
            // One reused inventory: a fresh full-system scan per poll is
            // needlessly expensive next to the whole workspace suite, and the
            // shell may need a refresh before it appears on a loaded runner.
            let mut system = fixture_system();
            let shell_start_time = {
                let start = Instant::now();
                loop {
                    system.refresh_processes_specifics(
                        sysinfo::ProcessesToUpdate::All,
                        true,
                        fixture_refresh_kind(),
                    );
                    if let Some(process) = system.process(sysinfo::Pid::from_u32(shell.id())) {
                        break process.start_time();
                    }
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "spawned shell {} never entered the process inventory",
                        shell.id()
                    );
                    thread::sleep(Duration::from_millis(50));
                }
            };
            Self {
                dir,
                shell,
                _serial: serial,
                system,
                shell_start_time,
            }
        }
        fn target(&self) -> agents::ShellTarget {
            agents::ShellTarget {
                session_id: "fixture".into(),
                generation: "generation".into(),
                pid: self.shell.id(),
                start_time: self.shell_start_time,
                foreground_pgid: None,
            }
        }
        fn inspect(&mut self) -> Vec<agents::TerminalPresence> {
            self.system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                fixture_refresh_kind(),
            );
            agents::inspect_shells(
                &agents::snapshot_processes(&self.system),
                &[self.target()],
                now(),
            )
        }
        /// Direct children of the shell for timeout diagnostics: pid, exe
        /// name, and argv[0] only, so command lines never leak into logs.
        fn children_debug(&mut self) -> String {
            self.system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                fixture_refresh_kind(),
            );
            let shell = self.shell.id();
            let mut children: Vec<String> = agents::snapshot_processes(&self.system)
                .iter()
                .filter(|proc| proc.parent == Some(shell))
                .map(|proc| {
                    format!(
                        "{} exe={} argv0={}",
                        proc.pid,
                        proc.exe_name,
                        proc.argv.first().map_or("?", String::as_str)
                    )
                })
                .collect();
            children.sort_unstable();
            if children.is_empty() {
                format!("<shell {shell} has no visible children>")
            } else {
                children.join(", ")
            }
        }
    }
    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = signals::signal_group(self.shell.id(), signals::ProcSignal::Kill);
            let _ = self.shell.wait();
            let _ = fs::remove_dir_all(self.dir.path());
        }
    }

    #[cfg(unix)]
    fn poll_kinds(fixture: &mut Fixture, wanted: &[&str]) -> agents::TerminalPresence {
        let start = Instant::now();
        loop {
            let presence = fixture.inspect()[0].clone();
            let mut kinds: Vec<_> = presence.agents.iter().map(|a| a.kind.as_str()).collect();
            kinds.sort_unstable();
            let mut expected = wanted.to_vec();
            expected.sort_unstable();
            if kinds == expected
                && presence.outcome == agents::PresenceOutcome::Verified
                && start.elapsed() > Duration::from_millis(50)
            {
                return presence;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "presence never became {wanted:?} within 30s: {presence:?}; shell children: {}",
                fixture.children_debug()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_processes_appear_and_remove_without_launching_agents() {
        let mut fixture = Fixture::spawn(&["codex"]);
        let presence = poll_kinds(&mut fixture, &["codex"]);
        let agent = &presence.agents[0];
        assert_eq!(agent.kind, "codex");
        assert!(!agent.foreground);
        // Exit removes live identity on the next successful inspection.
        signals::signal_process(agent.process.pid, signals::ProcSignal::Kill).unwrap();
        poll_kinds(&mut fixture, &[]);
    }

    #[cfg(unix)]
    #[test]
    fn sibling_agents_share_one_shell_without_merging() {
        let mut fixture = Fixture::spawn(&["claude", "pi"]);
        let presence = poll_kinds(&mut fixture, &["claude", "pi"]);
        assert_eq!(presence.agents.len(), 2);
        assert!(agents::preferred_agent(&presence.agents).is_none());
    }
}
