//! Versioned local protocol and persistent, renderer-independent models.
#![forbid(unsafe_code)]
pub mod agents;
pub mod appearance;
#[cfg(feature = "async-client")]
pub mod async_client;
#[cfg(feature = "async-client")]
pub mod async_process;
#[cfg(feature = "async-client")]
pub mod async_service;
pub mod crash;
pub mod generations;
pub mod git;
pub mod idle_close;
mod ipc;
pub mod metadata;
mod model;
pub mod recovery;
pub mod signals;
pub mod snapshot;
pub mod transport;
pub mod ui_control;
pub mod worktrees;

use anyhow::{Context, Result, bail, ensure};
pub use ipc::{
    Envelope, Request, Response, SnapshotHint, conditional_snapshot, connect, read_frame, rpc,
    sanitize_layout, write_frame,
};
pub use model::{
    AGENT_PRESENCE_CAPABILITY, Agent, AgentState, DIFF_CLOSE_SETTINGS_CAPABILITY, Dismissal,
    EditorMode, HookEvent, Lifecycle, METADATA_SETTINGS_CAPABILITY, NOTIFICATION_SOUND_CAPABILITY,
    NTFY_CAPABILITY, Notification, Project, Resume, ReviewMode, SCREEN_CAPABILITY,
    SHUTDOWN_IF_IDLE_CAPABILITY, STABLE_HELPER_CAPABILITY, Session, SessionKind, Settings, State,
    TERMINAL_NOTICES_CAPABILITY, TerminalNotice, WORKTREES_CAPABILITY,
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const NVIM_REVIEW_CAPABILITY: &str = "nvim-review-v1";
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
/// Retained agent/notification records are aggregated into every snapshot, so
/// they are bounded independently per generation. Pending attention is never
/// dropped by these caps.
pub const MAX_NOTIFICATIONS: usize = 512;
pub const MAX_AGENTS: usize = 512;
pub const MAX_RECENT_EVENTS: usize = 2048;
#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[must_use]
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
/// Find an executable without invoking a shell. Include standard GUI-launch paths.
pub fn find_executable(program: &str) -> Option<PathBuf> {
    let mut paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(base) = directories::BaseDirs::new() {
        paths.extend([".local/bin", ".opencode/bin", ".grok/bin"].map(|p| base.home_dir().join(p)));
    }
    find_executable_in(program, &paths)
}
/// Windows executable search stems for `program`: the name itself when it
/// already carries an extension, otherwise the name plus each `PATHEXT`
/// extension (`.exe`, `.bat`, ...). Plain `PATH` entries never include the
/// extension, so `nvim` must match `nvim.exe`.
#[cfg(windows)]
fn windows_candidate_names(program: &str) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        return vec![program.to_owned()];
    }
    let extensions = std::env::var_os("PATHEXT")
        .map(|value| {
            std::env::split_paths(&value)
                .filter_map(|p| {
                    let ext = p.to_str()?.trim().to_owned();
                    (!ext.is_empty()).then_some(ext)
                })
                .collect::<Vec<_>>()
        })
        .filter(|exts| !exts.is_empty())
        .unwrap_or_else(|| {
            [".COM", ".EXE", ".BAT", ".CMD"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        });
    extensions
        .iter()
        .map(|ext| format!("{program}{ext}"))
        .collect()
}

fn find_executable_in(program: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    #[cfg(windows)]
    let candidates: Vec<PathBuf> = if program.contains('/') || program.contains('\\') {
        windows_candidate_names(program)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        paths
            .iter()
            .flat_map(|p| {
                windows_candidate_names(program)
                    .into_iter()
                    .map(|name| p.join(name))
            })
            .collect()
    };
    #[cfg(not(windows))]
    let candidates = if program.contains('/') {
        vec![PathBuf::from(program)]
    } else {
        paths.iter().map(|p| p.join(program)).collect()
    };
    candidates.into_iter().find(|p| {
        p.metadata().is_ok_and(|m| {
            if !m.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    })
}
/// Sibling executable file name for this platform (`.exe` on Windows).
/// Idempotent: a stem that already carries the extension is returned unchanged
/// so `sibling_exe` stays correct even when handed a `bundled_exes()` entry.
#[must_use]
pub fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        if Path::new(stem)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            stem.into()
        } else {
            format!("{stem}.exe")
        }
    } else {
        stem.into()
    }
}
/// `executable`'s sibling with the platform file name applied.
#[must_use]
pub fn sibling_exe(executable: &Path, stem: &str) -> PathBuf {
    executable.with_file_name(exe_name(stem))
}
/// Both bundled sibling executables with the platform file names applied.
#[must_use]
pub fn bundled_exes() -> [String; 2] {
    [exe_name("terminator-daemon"), exe_name("terminator-hook")]
}
/// Raw subprocess-output bytes as an `OsStr`. Unix paths are arbitrary bytes;
/// Windows paths must be valid WTF-8, so anything else becomes lossy UTF-8.
#[must_use]
pub fn os_name(bytes: &[u8]) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(bytes).to_os_string()
    }
    #[cfg(not(unix))]
    {
        String::from_utf8_lossy(bytes).into_owned().into()
    }
}
/// `OsStr` as bytes for comparison against subprocess output. Lossy on
/// Windows, where paths cannot hold arbitrary bytes.
#[must_use]
pub fn os_bytes(value: &std::ffi::OsStr) -> std::borrow::Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::borrow::Cow::Borrowed(value.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::borrow::Cow::Owned(value.to_string_lossy().into_owned().into_bytes())
    }
}
/// Check the current user's ability to execute a regular file, including ACLs.
#[must_use]
pub fn executable_available(path: &Path) -> bool {
    #[cfg(unix)]
    {
        path.is_file() && rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}
pub fn default_shell() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        ["zsh", "bash", "sh"]
            .into_iter()
            .find_map(find_executable)
            .context("No zsh, bash, or sh executable found")
    }
    #[cfg(not(unix))]
    {
        ["pwsh", "powershell", "cmd"]
            .into_iter()
            .find_map(find_executable)
            .or_else(|| std::env::var_os("COMSPEC").map(PathBuf::from))
            .context("No pwsh, powershell, or cmd executable found")
    }
}
#[must_use]
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paths {
    pub data: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("TERMINATOR_DATA_DIR") {
            let mut paths = Self::at(PathBuf::from(path));
            if let Some(runtime) = std::env::var_os("TERMINATOR_RUNTIME_DIR") {
                paths.runtime = runtime.into();
            }
            return Ok(paths);
        }
        let dirs = directories::ProjectDirs::from("dev", "terminator", "Terminator")
            .context("No user data directory")?;
        // Unix sockets have short path limits, particularly on macOS.
        let home = directories::BaseDirs::new().context("No home directory")?;
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        home.home_dir().hash(&mut h);
        let data: PathBuf = dirs.data_local_dir().into();
        #[cfg(unix)]
        let runtime = PathBuf::from(format!("/tmp/terminator-{:x}", h.finish()));
        // Windows uses a TCP-loopback port file, so the runtime dir only
        // needs to be private, not short.
        #[cfg(not(unix))]
        let runtime = std::env::temp_dir().join(format!("terminator-{:x}", h.finish()));
        Ok(Self { data, runtime })
    }
    #[must_use]
    pub fn at(data: PathBuf) -> Self {
        Self {
            runtime: data.join("run"),
            data,
        }
    }
    pub fn init(&self) -> Result<()> {
        for path in [&self.data, &self.runtime, &self.history_dir()] {
            if path.exists() {
                ensure!(
                    !fs::symlink_metadata(path)?.file_type().is_symlink(),
                    "Refusing symlink data directory"
                );
            }
            fs::create_dir_all(path)?;
            #[cfg(unix)]
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(unix)]
        ensure!(
            self.socket().as_os_str().len() < 100,
            "Runtime path too long for Unix socket; use a shorter data directory"
        );
        Ok(())
    }
    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("daemon.sock")
    }
    #[must_use]
    pub fn auth(&self) -> PathBuf {
        self.runtime.join("auth")
    }
    #[must_use]
    pub fn history_dir(&self) -> PathBuf {
        self.data.join("history")
    }
    #[must_use]
    pub fn crash_dir(&self) -> PathBuf {
        self.data.join("crashes")
    }
    #[must_use]
    pub fn editor_socket(&self, session: &str) -> PathBuf {
        self.runtime
            .join(format!("{}.nvim", session.get(..8).unwrap_or(session)))
    }
    pub fn token(&self) -> Result<String> {
        Ok(fs::read_to_string(self.auth())
            .with_context(|| {
                format!(
                    "Cannot read session service authentication file {}",
                    self.auth().display()
                )
            })?
            .trim()
            .into())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".terminator-{}.tmp", id()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

pub mod process;
pub use process::{CommandOptions, run_command, spawn_session_leader};
/// Legacy callers inspect the exit status themselves.
pub fn bounded_output(
    cmd: std::process::Command,
    timeout: Duration,
) -> Result<std::process::Output> {
    run_command(
        cmd,
        CommandOptions {
            timeout,
            accepted_exit_codes: None,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn shell_preference_skips_missing_and_nonexecutable_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![dir.path().to_path_buf()];
        let choose = || {
            ["zsh", "bash", "sh"]
                .into_iter()
                .find_map(|name| find_executable_in(name, &paths))
        };
        assert!(choose().is_none());
        for name in ["sh", "bash", "zsh"] {
            let p = dir.path().join(name);
            fs::write(&p, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(choose(), Some(p));
        }
        fs::set_permissions(dir.path().join("zsh"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(choose(), Some(dir.path().join("bash")));
        fs::remove_file(dir.path().join("bash")).unwrap();
        assert_eq!(choose(), Some(dir.path().join("sh")));
    }

    #[test]
    fn bundled_names_already_carry_the_platform_suffix() {
        for name in bundled_exes() {
            assert_eq!(exe_name(&name), name);
            assert_eq!(
                sibling_exe(Path::new("/tmp/terminator"), &name),
                Path::new("/tmp/terminator").with_file_name(&name)
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_lookup_finds_bare_names_with_executable_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![dir.path().to_path_buf()];
        assert!(find_executable_in("nvim", &paths).is_none());
        std::fs::write(dir.path().join("nvim.exe"), b"fixture").unwrap();
        assert_eq!(
            find_executable_in("nvim", &paths).map(|path| path.canonicalize().unwrap()),
            Some(dir.path().join("nvim.exe").canonicalize().unwrap())
        );
        assert_eq!(
            find_executable_in("nvim.exe", &paths),
            Some(dir.path().join("nvim.exe"))
        );
    }

    /// Keep Unix socket paths short; Windows uses the system temporary directory.
    pub(crate) fn tempdir(prefix: &str) -> tempfile::TempDir {
        #[cfg(unix)]
        let root = PathBuf::from("/tmp");
        #[cfg(not(unix))]
        let root = std::env::temp_dir();
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(root)
            .unwrap()
    }

    pub(crate) fn setup() -> State {
        let mut s = State::default();
        s.sessions.push(Session {
            review: false,
            id: "s".into(),
            project_id: "p".into(),
            label: "shell".into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: id(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        });
        s
    }

    #[test]
    fn terminal_notifications_never_create_or_change_agents_and_are_bounded() {
        let mut state = setup();
        state
            .terminal_notice("s", "Terminal title", "permission needed")
            .unwrap();
        assert!(state.agents.is_empty());
        assert!(state.notifications.is_empty());
        assert!(
            state
                .terminal_notice("s", "Terminal title", "permission needed")
                .unwrap()
                .is_none()
        );
        assert!(state.terminal_notice("missing", "test", "test").is_err());
        for i in 0..200 {
            state
                .terminal_notice("s", &format!("{i}"), "hello")
                .unwrap();
        }
        assert_eq!(state.terminal_notices.len(), 128);
    }
    pub(crate) fn event(n: u64, state: AgentState) -> HookEvent {
        HookEvent {
            protocol_version: 1,
            event_id: format!("e{n}"),
            terminal_session_id: "s".into(),
            agent_invocation_id: "a".into(),
            agent_kind: "custom".into(),
            provider_session_id: None,
            state,
            request_id: Some("req".into()),
            sequence: Some(n),
            summary: "Need input".into(),
            details: String::new(),
            resume: None,
            process: None,
        }
    }

    #[test]
    fn ntfy_eligible_notifications_suppress_duplicate_and_stale_hooks() {
        let mut state = setup();
        state.settings.ntfy_enabled = true;
        state.settings.ntfy_channel = "fixture".into();
        assert!(
            state
                .apply_hook(event(2, AgentState::WaitingInput))
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .apply_hook(event(2, AgentState::WaitingInput))
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .apply_hook(event(1, AgentState::Completed))
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .apply_hook(event(3, AgentState::WaitingInput))
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .apply_hook(event(4, AgentState::Completed))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn dismissal_does_not_resume_agent() {
        let mut s = setup();
        s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
        s.focus("s");
        assert!(s.notifications[0].dismissed);
        assert_eq!(s.agents[0].state, AgentState::WaitingInput);
        s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
        assert_eq!(s.notifications.len(), 1);
    }

    #[test]
    fn stale_events_and_other_invocations() {
        let mut s = setup();
        s.apply_hook(event(3, AgentState::Running)).unwrap();
        s.apply_hook(event(2, AgentState::WaitingPermission))
            .unwrap();
        assert!(s.notifications.is_empty());
        let mut e = event(4, AgentState::Completed);
        e.agent_invocation_id = "old-agent".into();
        s.apply_hook(e).unwrap();
        assert_eq!(s.agents[0].state, AgentState::Running);
    }

    #[test]
    fn recovery_does_not_relaunch() {
        let mut s = setup();
        s.recover();
        assert_eq!(s.sessions[0].lifecycle, Lifecycle::Interrupted);
        assert!(s.sessions[0].pid.is_none());
    }

    #[test]
    fn recovery_preserves_history_loss_but_discards_previous_runtime_health() {
        let mut state = setup();
        state.sessions[0].truncated = true;
        state.degraded = Some("Output storage queue saturated; some history was not saved".into());
        state.daemon_executable = Some("/removed/daemon".into());
        state.attachment_helper_executable = Some("/removed/helper".into());
        state.attachment_helper_available = Some(false);
        state.recover();
        assert!(state.sessions[0].truncated);
        assert!(state.degraded.is_none());
        assert!(state.daemon_executable.is_none());
        assert!(state.attachment_helper_executable.is_none());
        assert!(state.attachment_helper_available.is_none());
    }

    #[test]
    fn shell_quote_is_literal() {
        assert_eq!(quote("a'b$(x)"), "'a'\\''b$(x)'");
    }

    fn agent_for(session: &str, resume: bool) -> Agent {
        Agent {
            invocation_id: format!("agent-{session}"),
            session_id: session.into(),
            kind: "codex".into(),
            provider_session_id: resume.then(|| "provider-1".into()),
            state: AgentState::Stopped,
            sequence: None,
            updated: 0,
            resume: resume.then(|| Resume {
                program: "codex".into(),
                args: vec!["resume".into(), "provider-1".into()],
            }),
            process: None,
        }
    }

    #[test]
    fn prune_keeps_only_live_or_resumable_ended_sessions() {
        let mut s = State::default();
        s.sessions
            .push(ended_session("plain-ended", Lifecycle::Ended));
        s.sessions
            .push(ended_session("plain-interrupted", Lifecycle::Interrupted));
        s.sessions
            .push(ended_session("resumable-ended", Lifecycle::Ended));
        s.sessions
            .push(ended_session("agent-without-resume", Lifecycle::Ended));
        let mut live = ended_session("live", Lifecycle::Running);
        live.lifecycle = Lifecycle::Running;
        s.sessions.push(live);
        s.agents.push(agent_for("resumable-ended", true));
        s.agents.push(agent_for("agent-without-resume", false));
        s.notifications.push(Notification {
            id: "n".into(),
            session_id: "plain-ended".into(),
            invocation_id: "x".into(),
            request_id: None,
            state: AgentState::Stopped,
            summary: String::new(),
            details: String::new(),
            created: 0,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        });
        assert!(s.session_has_resume("resumable-ended"));
        assert!(!s.session_has_resume("plain-ended"));
        assert!(!s.session_has_resume("agent-without-resume"));
        let mut removed = s.prune_non_resumable_ended();
        removed.sort();
        assert_eq!(
            removed,
            vec![
                "agent-without-resume".to_string(),
                "plain-ended".to_string(),
                "plain-interrupted".to_string()
            ]
        );
        let kept: Vec<_> = s
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect();
        assert!(kept.contains(&"resumable-ended".to_string()));
        assert!(kept.contains(&"live".to_string()));
        assert!(s.notifications.is_empty());
        assert!(s.agents.iter().all(|a| a.session_id != "plain-ended"));
    }

    #[test]
    fn compact_history_bounds_records_and_keeps_pending_attention() {
        fn notice(id: String, settled: bool) -> Notification {
            Notification {
                id,
                session_id: "s".into(),
                invocation_id: "x".into(),
                request_id: None,
                state: AgentState::Completed,
                summary: String::new(),
                details: String::new(),
                created: 0,
                read: settled,
                dismissed: settled,
                resolved: settled,
                snoozed_until: 0,
            }
        }
        let mut s = State::default();
        s.notifications.push(notice("pending".into(), false));
        for i in 0..(MAX_NOTIFICATIONS + 200) {
            s.notifications.push(notice(format!("n{i}"), true));
        }
        // Live and resumable agents must survive even when older than the cap.
        let mut running = agent_for("live-agent", false);
        running.state = AgentState::Running;
        s.agents.push(running);
        s.agents.push(agent_for("resumable-agent", true));
        for i in 0..(MAX_AGENTS + 200) {
            s.agents.push(agent_for(&format!("stopped-{i}"), false));
        }
        s.recent_events.push("event".into());
        s.compact_history();
        assert!(s.recent_events.is_empty());
        assert!(s.notifications.iter().any(|n| n.id == "pending"));
        assert!(s.notifications.len() <= MAX_NOTIFICATIONS + 1);
        assert!(s.agents.iter().any(|a| a.session_id == "live-agent"));
        assert!(s.agents.iter().any(|a| a.session_id == "resumable-agent"));
        assert!(s.agents.len() <= MAX_AGENTS + 2);
    }

    pub(crate) fn ended_session(id: &str, lifecycle: Lifecycle) -> Session {
        Session {
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        }
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::tests::{ended_session, event, setup};
    use super::*;

    #[test]
    fn legacy_snapshots_do_not_advertise_new_daemon_features() {
        let legacy: State = serde_json::from_value(serde_json::json!({"revision":7})).unwrap();
        assert!(legacy.capabilities.is_empty());
        assert!(legacy.daemon_version.is_none());
        let new = State {
            capabilities: vec![NVIM_REVIEW_CAPABILITY.into()],
            ..State::default()
        };
        let encoded = serde_json::to_value(&new).unwrap();
        #[derive(Deserialize)]
        struct LegacyState {
            revision: u64,
        }
        assert_eq!(
            serde_json::from_value::<LegacyState>(encoded.clone())
                .unwrap()
                .revision,
            0
        );
        assert_eq!(
            serde_json::from_value::<State>(encoded)
                .unwrap()
                .capabilities,
            new.capabilities
        );
    }

    #[test]
    fn restarting_changes_hint_even_if_revision_is_equal() {
        let mut state = State::default();
        state.recover();
        let previous = state.snapshot_hint();
        state.recover();
        state.revision = previous.revision;
        assert_ne!(previous, state.snapshot_hint());
    }

    fn presence_for(session: &str, kind: &str, pid: u32, start: u64) -> State {
        let mut state = setup();
        state.presence.push(agents::TerminalPresence {
            session_id: session.into(),
            generation: state.sessions[0].generation.clone(),
            agents: vec![agents::DetectedAgent {
                kind: kind.into(),
                process: agents::ProcessIdentity {
                    pid,
                    start_time: start,
                },
                foreground: true,
            }],
            outcome: agents::PresenceOutcome::Verified,
            observed_at: now(),
        });
        state
    }

    #[test]
    fn hook_process_identity_links_only_to_a_verified_live_observation() {
        let mut state = presence_for("s", "codex", 4242, 777);
        let mut linked = event(1, AgentState::Running);
        linked.agent_kind = "codex".into();
        linked.process = Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777,
        });
        state.apply_hook(linked).unwrap();
        assert_eq!(
            state.agents[0].process,
            Some(agents::ProcessIdentity {
                pid: 4242,
                start_time: 777
            })
        );
        // Wrong PID, recycled start time, and wrong kind never link.
        for (pid, start, kind) in [
            (9999, 777, "codex"),
            (4242, 778, "codex"),
            (4242, 777, "claude"),
        ] {
            let mut state = presence_for("s", "codex", 4242, 777);
            let mut e = event(1, AgentState::Running);
            e.agent_kind = kind.into();
            e.event_id = format!("e-{pid}-{start}-{kind}");
            e.process = Some(agents::ProcessIdentity {
                pid,
                start_time: start,
            });
            state.apply_hook(e).unwrap();
            assert!(
                state.agents[0].process.is_none(),
                "unverified identity must not link: {pid}/{start}/{kind}"
            );
            // ... but lifecycle delivery is preserved.
            assert_eq!(state.agents[0].state, AgentState::Running);
        }
        // Stale observations cannot verify new links.
        let mut state = presence_for("s", "codex", 4242, 777);
        state.presence[0].observed_at = now().saturating_sub(30);
        let mut e = event(1, AgentState::Running);
        e.agent_kind = "codex".into();
        e.process = Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777,
        });
        state.apply_hook(e).unwrap();
        assert!(state.agents[0].process.is_none());
    }

    #[test]
    fn hook_without_process_identity_preserves_existing_delivery() {
        let mut state = presence_for("s", "codex", 4242, 777);
        let delivered = state
            .apply_hook(event(1, AgentState::WaitingInput))
            .unwrap();
        assert!(delivered.is_some());
        assert!(state.agents[0].process.is_none());
        assert_eq!(state.agents[0].state, AgentState::WaitingInput);
        assert_eq!(state.notifications.len(), 1);
    }

    #[test]
    fn new_invocation_cannot_inherit_an_earlier_invocations_process_link() {
        let mut state = presence_for("s", "codex", 4242, 777);
        let mut first = event(1, AgentState::Running);
        first.agent_kind = "codex".into();
        first.agent_invocation_id = "first".into();
        first.process = Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777,
        });
        state.apply_hook(first).unwrap();
        assert!(state.agents[0].process.is_some());
        let mut second = event(2, AgentState::Running);
        second.agent_kind = "codex".into();
        second.agent_invocation_id = "second".into();
        second.event_id = "e-second".into();
        second.process = Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777,
        });
        // The detected process left; the observation is empty now.
        state.presence[0].agents.clear();
        state.apply_hook(second).unwrap();
        assert_eq!(state.agents.len(), 2);
        assert!(state.agents[1].process.is_none());
    }

    #[test]
    fn presence_observations_never_emit_lifecycle_events_or_notifications() {
        let state = presence_for("s", "codex", 4242, 777);
        assert!(state.agents.is_empty());
        assert!(state.notifications.is_empty());
        assert!(state.recent_events.is_empty());
    }

    #[test]
    fn presence_fields_are_additive_and_old_snapshots_parse_cleanly() {
        let legacy: State = serde_json::from_value(serde_json::json!({
            "revision": 3,
            "agents": [{"invocation_id": "a", "session_id": "s", "kind": "codex",
                        "provider_session_id": null, "state": "running",
                        "sequence": null, "updated": 1, "resume": null}],
        }))
        .unwrap();
        assert!(legacy.presence.is_empty());
        assert!(legacy.agents[0].process.is_none());
        let legacy_event: HookEvent = serde_json::from_value(serde_json::json!({
            "protocol_version": 1, "event_id": "e", "terminal_session_id": "s",
            "agent_invocation_id": "a", "agent_kind": "codex",
            "provider_session_id": null, "state": "running", "request_id": null,
            "sequence": null, "summary": "hi", "details": "", "resume": null,
        }))
        .unwrap();
        assert!(legacy_event.process.is_none());
        // New snapshots round-trip through the additive fields.
        let state = presence_for("s", "codex", 4242, 777);
        let encoded = serde_json::to_value(&state).unwrap();
        assert_eq!(encoded["presence"][0]["agents"][0]["kind"], "codex");
        let restored: State = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored.presence, state.presence);
    }

    #[test]
    fn recovery_clears_presence_and_process_links_without_relaunching() {
        let mut state = presence_for("s", "codex", 4242, 777);
        let mut e = event(1, AgentState::Running);
        e.agent_kind = "codex".into();
        e.process = Some(agents::ProcessIdentity {
            pid: 4242,
            start_time: 777,
        });
        state.apply_hook(e).unwrap();
        assert!(state.agents[0].process.is_some());
        state.recover();
        assert!(state.presence.is_empty());
        assert!(state.agents[0].process.is_none());
        assert_eq!(state.sessions[0].lifecycle, Lifecycle::Interrupted);
    }

    #[test]
    fn prune_drops_presence_for_removed_sessions() {
        let mut s = State::default();
        s.sessions.push(ended_session("gone", Lifecycle::Ended));
        s.presence.push(agents::TerminalPresence {
            session_id: "gone".into(),
            generation: "g".into(),
            agents: Vec::new(),
            outcome: agents::PresenceOutcome::Verified,
            observed_at: now(),
        });
        s.prune_non_resumable_ended();
        assert!(s.presence.is_empty());
    }
}

#[cfg(test)]
mod executable_health_tests {
    #[cfg(unix)]
    use super::*;
    #[cfg(unix)]
    #[test]
    fn missing_non_executable_and_directory_helpers_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("helper");
        assert!(!executable_available(&file));
        assert!(!executable_available(dir.path()));
        fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!executable_available(&file));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(executable_available(&file));
        fs::remove_file(&file).unwrap();
        assert!(!executable_available(&file));
    }
}
