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
pub mod remote;
pub mod signals;
pub mod snapshot;
pub mod ssh;
pub mod transport;
pub mod ui_control;
pub mod vnc;
pub mod worktrees;

use anyhow::{Context, Result, bail, ensure};
pub use error::{AtomicWriteError, ErrorChain, ErrorReport, NoShellError, PathsError};
pub use ipc::{
    Envelope, Request, Response, SnapshotHint, conditional_snapshot, connect, read_frame, rpc,
    sanitize_layout, write_frame,
};
pub use model::{
    AGENT_PRESENCE_CAPABILITY, Agent, AgentState, DIFF_CLOSE_SETTINGS_CAPABILITY, Dismissal,
    EditorMode, HookEvent, Lifecycle, METADATA_SETTINGS_CAPABILITY, NOTIFICATION_SOUND_CAPABILITY,
    NTFY_CAPABILITY, Notification, Project, Resume, ReviewMode, SCREEN_CAPABILITY,
    SHUTDOWN_IF_IDLE_CAPABILITY, STABLE_HELPER_CAPABILITY, Session, SessionKind, Settings,
    SettingsError, State, StateError, TERMINAL_NOTICES_CAPABILITY, TerminalNotice, VNC_CAPABILITY,
    VNC_REGIONS_CAPABILITY, WORKTREES_CAPABILITY,
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
/// Unix socket paths must stay below `sun_path` (104 bytes on macOS).
const SOCKET_PATH_LIMIT: usize = 100;
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
    // Settings fields are typed by hand, so accept a leading `~/`.
    if let Some(rest) = program.strip_prefix("~/") {
        let base = directories::BaseDirs::new()?;
        let path = base.home_dir().join(rest);
        return is_executable_file(&path).then_some(path);
    }
    let mut paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(base) = directories::BaseDirs::new() {
        paths.extend(
            [".local/bin", ".cargo/bin", ".opencode/bin", ".grok/bin"]
                .map(|p| base.home_dir().join(p)),
        );
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
    candidates.into_iter().find(|p| is_executable_file(p))
}

fn is_executable_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| {
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
pub fn default_shell() -> Result<PathBuf, NoShellError> {
    #[cfg(unix)]
    {
        ["zsh", "bash", "sh"]
            .into_iter()
            .find_map(find_executable)
            .ok_or(NoShellError)
    }
    #[cfg(not(unix))]
    {
        ["pwsh", "powershell", "cmd"]
            .into_iter()
            .find_map(find_executable)
            .or_else(|| std::env::var_os("COMSPEC").map(PathBuf::from))
            .ok_or(NoShellError)
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
    pub fn discover() -> Result<Self, PathsError> {
        if let Some(path) = std::env::var_os("TERMINATOR_DATA_DIR") {
            let mut paths = Self::at(PathBuf::from(path));
            if let Some(runtime) = std::env::var_os("TERMINATOR_RUNTIME_DIR") {
                paths.runtime = runtime.into();
            }
            return Ok(paths);
        }
        let dirs = directories::ProjectDirs::from("dev", "terminator", "Terminator")
            .ok_or(PathsError::NoDataDirectory)?;
        // Unix sockets have short path limits, particularly on macOS.
        let home = directories::BaseDirs::new().ok_or(PathsError::NoHomeDirectory)?;
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
    pub fn init(&self) -> Result<(), PathsError> {
        for path in [&self.data, &self.runtime, &self.history_dir()] {
            if path.exists() && fs::symlink_metadata(path)?.file_type().is_symlink() {
                return Err(PathsError::SymlinkDataDirectory { path: path.clone() });
            }
            fs::create_dir_all(path)?;
            #[cfg(unix)]
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(unix)]
        if self.socket().as_os_str().len() >= SOCKET_PATH_LIMIT {
            return Err(PathsError::SocketPathTooLong {
                socket: self.socket(),
            });
        }
        Ok(())
    }
    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("daemon.sock")
    }
    /// OpenSSH multiplexing socket shared by every connection with the same
    /// endpoint and trust settings. `None` when the runtime directory is too
    /// long for a Unix socket, or on Windows where OpenSSH cannot multiplex;
    /// callers then connect without reuse.
    #[must_use]
    pub fn ssh_control(&self, ssh: &ssh::SshOptions) -> Option<PathBuf> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            &ssh.host,
            ssh.port,
            &ssh.username,
            &ssh.identity_file,
            &ssh.known_hosts_file,
        )
            .hash(&mut h);
        let path = self
            .runtime
            .join(format!("ssh-{:08x}", h.finish() & 0xffff_ffff));
        // OpenSSH binds a temporary `<path>.<16 random chars>` before renaming.
        (cfg!(unix) && path.as_os_str().len().saturating_add(17) < SOCKET_PATH_LIMIT)
            .then_some(path)
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
    pub fn token(&self) -> Result<String, PathsError> {
        Ok(fs::read_to_string(self.auth())
            .map_err(|source| PathsError::Token {
                path: self.auth(),
                source,
            })?
            .trim()
            .into())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AtomicWriteError> {
    let parent = path
        .parent()
        .ok_or_else(|| AtomicWriteError::MissingParent {
            path: path.to_path_buf(),
        })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".terminator-{}.tmp", id()));
    let result = (|| -> Result<(), AtomicWriteError> {
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

mod error;
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
mod tests;

#[cfg(test)]
mod snapshot_tests;

#[cfg(test)]
mod executable_health_tests;
