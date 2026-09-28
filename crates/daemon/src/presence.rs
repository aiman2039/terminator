//! Live agent-presence inspection. One background pass every two seconds
//! batches all owned live shell sessions against a single process inventory.
//! Observations never create lifecycle events, notifications, or resume
//! commands, and never touch SQLite.
use super::*;
use terminator_core::agents;

/// One target per owned live shell session, with its PTY foreground group.
fn targets(shared: &Shared) -> Vec<agents::ShellTarget> {
    let live: Vec<(String, String, u32, u64)> = shared
        .state
        .lock()
        .unwrap()
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live() && s.kind == SessionKind::Shell)
        .filter_map(|s| {
            s.pid
                .map(|pid| (s.id.clone(), s.generation.clone(), pid, s.created))
        })
        .collect();
    let runtimes = shared.sessions.lock().unwrap();
    live.into_iter()
        .map(|(id, generation, pid, created)| {
            let foreground_pgid = runtimes
                .get(&id)
                .and_then(|runtime| runtime.lock().unwrap().master.process_group_leader())
                .and_then(|pgid| u32::try_from(pgid).ok())
                .filter(|pgid| *pgid > 1);
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
        let mut state = shared.state.lock().unwrap();
        if !state.presence.is_empty() {
            state.presence.clear();
            state.revision += 1;
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
    let mut state = shared.state.lock().unwrap();
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
        state.revision += 1;
    }
}

fn update_presence(state: &mut State, presence: Vec<agents::TerminalPresence>) {
    if presence_changed(&state.presence, &presence) {
        state.revision += 1;
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
    /// SIGKILLed by macOS code-signing enforcement.)
    #[test]
    fn presence_fixture_sleeper() {
        if std::env::var_os("TERMINATOR_FIXTURE_SLEEPER").is_none() {
            return;
        }
        std::thread::sleep(Duration::from_secs(60));
    }

    /// Fake agent executables: renamed test-binary copies running the sleeper.
    struct Fixture {
        dir: tempfile::TempDir,
        shell: std::process::Child,
    }
    impl Fixture {
        fn spawn(agents: &[&str]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let current = std::env::current_exe().unwrap();
            let mut launches = Vec::new();
            for agent in agents {
                let exe = dir.path().join(agent);
                fs::copy(&current, &exe).unwrap();
                fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
                launches.push(format!(
                    "TERMINATOR_FIXTURE_SLEEPER=1 '{}' presence_fixture_sleeper --exact --nocapture &",
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
            Self { dir, shell }
        }
        fn target(&self) -> agents::ShellTarget {
            let pid = self.shell.id();
            let system = sysinfo::System::new_with_specifics(
                sysinfo::RefreshKind::nothing().with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_exe(sysinfo::UpdateKind::Always)
                        .with_cmd(sysinfo::UpdateKind::Always),
                ),
            );
            let start_time = system
                .process(sysinfo::Pid::from_u32(pid))
                .map(|p| p.start_time())
                .unwrap();
            agents::ShellTarget {
                session_id: "fixture".into(),
                generation: "generation".into(),
                pid,
                start_time,
                foreground_pgid: None,
            }
        }
        fn inspect(&self) -> Vec<agents::TerminalPresence> {
            let system = sysinfo::System::new_with_specifics(
                sysinfo::RefreshKind::nothing().with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_exe(sysinfo::UpdateKind::Always)
                        .with_cmd(sysinfo::UpdateKind::Always),
                ),
            );
            agents::inspect_shells(
                &agents::snapshot_processes(&system),
                &[self.target()],
                now(),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = signals::signal_group(self.shell.id(), signals::ProcSignal::Kill);
            let _ = self.shell.wait();
            let _ = fs::remove_dir_all(self.dir.path());
        }
    }

    fn poll_kinds(fixture: &Fixture, wanted: &[&str]) -> agents::TerminalPresence {
        let start = Instant::now();
        loop {
            let presence = &fixture.inspect()[0];
            let mut kinds: Vec<_> = presence.agents.iter().map(|a| a.kind.as_str()).collect();
            kinds.sort_unstable();
            let mut expected = wanted.to_vec();
            expected.sort_unstable();
            if kinds == expected
                && presence.outcome == agents::PresenceOutcome::Verified
                && start.elapsed() > Duration::from_millis(50)
            {
                return presence.clone();
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "presence never became {wanted:?}: {presence:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn real_processes_appear_and_remove_without_launching_agents() {
        let fixture = Fixture::spawn(&["codex"]);
        let presence = poll_kinds(&fixture, &["codex"]);
        let agent = &presence.agents[0];
        assert_eq!(agent.kind, "codex");
        assert!(!agent.foreground);
        // Exit removes live identity on the next successful inspection.
        signals::signal_process(agent.process.pid, signals::ProcSignal::Kill).unwrap();
        poll_kinds(&fixture, &[]);
    }

    #[test]
    fn sibling_agents_share_one_shell_without_merging() {
        let fixture = Fixture::spawn(&["claude", "pi"]);
        let presence = poll_kinds(&fixture, &["claude", "pi"]);
        assert_eq!(presence.agents.len(), 2);
        assert!(agents::preferred_agent(&presence.agents).is_none());
    }
}
