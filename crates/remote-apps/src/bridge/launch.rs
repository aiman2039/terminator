//! `--ensure`: find or start this user's wprsd, like vncserver finds or
//! starts a display. The display survives the bridge and the SSH channel;
//! only the bridge (the viewer) comes and goes.
//!
//! Files live in one private directory (0700, never on NFS):
//! `/tmp/terminator-remote-<uid>`. Not `$XDG_RUNTIME_DIR`: systemd removes
//! it when the user's last session ends, and wprsd must outlive that.
//!
//! - `wprsd.json`: the running wprsd, written as soon as it answers, so a
//!   bridge that dies early never leaves a wprsd nobody can find.
//! - `session.json`: the same, for shells; written only after a bridge gave
//!   wprsd an output, so a shell that reads it gets a display that maps
//!   windows (xwayland-xdg-shell ignores windows mapped before an output).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// First X display number tried; low numbers belong to real and VNC servers.
const FIRST_DISPLAY: u32 = 100;
const LAST_DISPLAY: u32 = 999;
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// With a wrapper the first start may download wprs (nix-portable).
const WRAPPER_START_TIMEOUT: Duration = Duration::from_mins(10);
const X11_DIR: &str = "/tmp/.X11-unix";

/// What a shell needs to show apps in Terminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub display: String,
    pub wayland_display: String,
    pub socket: PathBuf,
    pub xauthority: PathBuf,
    pub pid: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("{0} not found (install Terminator's remote-app tools on this host)")]
    Missing(&'static str),
    #[error("Xwayland not found on PATH (install the xwayland package)")]
    XwaylandMissing,
    #[error("private directory {0} is not safe: {1}")]
    UnsafeDir(PathBuf, &'static str),
    #[error("no free X display between :{FIRST_DISPLAY} and :{LAST_DISPLAY}")]
    NoDisplay,
    #[error("wprsd did not start in time; log: {log}")]
    StartTimeout { log: String },
    #[error("wprsd exited at start; log: {log}")]
    StartFailed { log: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The private directory, created when missing.
pub fn runtime_dir() -> Result<PathBuf, LaunchError> {
    let uid = nix::unistd::getuid().as_raw();
    let dir = std::env::var_os("TERMINATOR_REMOTE_DIR").map_or_else(
        || PathBuf::from(format!("/tmp/terminator-remote-{uid}")),
        PathBuf::from,
    );
    private_dir(&dir)?;
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.is_dir() {
        return Err(LaunchError::UnsafeDir(dir, "not a directory"));
    }
    if meta.uid() != uid {
        return Err(LaunchError::UnsafeDir(dir, "owned by another user"));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(LaunchError::UnsafeDir(dir, "readable by other users"));
    }
    Ok(dir)
}

/// The running wprsd recorded in `dir`, if it still answers.
pub fn live_session(dir: &Path) -> Option<Session> {
    let text = fs::read_to_string(dir.join(WPRSD_FILE)).ok()?;
    let session: Session = serde_json::from_str(&text).ok()?;
    UnixStream::connect(&session.socket).ok()?;
    Some(session)
}

/// Shells may use the display now (it has an output): writes
/// `session.json` unless it already describes `session`.
pub fn publish_for_shells(dir: &Path, session: &Session) -> Result<(), LaunchError> {
    write_json(dir, SHELL_FILE, session)
}

const WPRSD_FILE: &str = "wprsd.json";
const SHELL_FILE: &str = "session.json";

fn write_json(dir: &Path, name: &str, session: &Session) -> Result<(), LaunchError> {
    let json = serde_json::to_vec_pretty(session).map_err(io::Error::other)?;
    Ok(write_private(dir, name, &json)?)
}

/// Replaces `dir/name` with `bytes` atomically (a 0600 temporary file, then
/// a rename), so readers never see half a file.
pub(super) fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!("{name}.tmp"));
    let mut file = private_file(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, dir.join(name))
}

/// Starts wprsd with Xwayland on a free display, detached from this process
/// group so the SSH channel closing does not stop it. Returns when the
/// wprsd socket and the X socket exist.
pub fn start(dir: &Path) -> Result<Session, LaunchError> {
    let tools = Tools::find()?;
    let number = free_display()?;
    let display = format!(":{number}");
    ensure_x11_dir()?;
    let xauthority = dir.join("Xauthority");
    write_cookie(&xauthority, number)?;
    let shim = xwayland_shim(dir, &xauthority)?;
    let socket = dir.join("wprs.sock");
    let _ = fs::remove_file(&socket);
    let log_path = dir.join("wprsd.log");
    let log = private_file(&log_path)?;
    let args = [
        format!("--socket={}", socket.display()),
        "--enable-xwayland=true".to_owned(),
        format!("--xwayland-xdg-shell-path={}", tools.xdg_shell),
        format!("--xwayland-xdg-shell-args=--display={number}"),
        "--stderr-log-level=warn".to_owned(),
    ];
    let mut child = wprsd_command(&tools, &args, shim)?
        .env("XDG_RUNTIME_DIR", dir)
        .env("XAUTHORITY", &xauthority)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let timeout = if tools.wrapper.is_some() {
        WRAPPER_START_TIMEOUT
    } else {
        START_TIMEOUT
    };
    let deadline = Instant::now().checked_add(timeout);
    loop {
        if child.try_wait()?.is_some() {
            return Err(LaunchError::StartFailed {
                log: tail(&log_path),
            });
        }
        let x_socket = Path::new(X11_DIR).join(format!("X{number}"));
        if x_socket.exists() && UnixStream::connect(&socket).is_ok() {
            break;
        }
        if deadline.is_none_or(|d| Instant::now() >= d) {
            let _ = child.kill();
            return Err(LaunchError::StartTimeout {
                log: tail(&log_path),
            });
        }
        thread::sleep(Duration::from_millis(100));
    }
    let session = Session {
        display,
        wayland_display: dir.join("wprs-0").display().to_string(),
        socket,
        xauthority,
        pid: child.id(),
    };
    write_json(dir, WPRSD_FILE, &session)?;
    Ok(session)
}

struct Tools {
    /// See [`WRAPPER_ENV`].
    wrapper: Option<String>,
    wprsd: PathBuf,
    /// A path, or a name the wrapper's PATH resolves.
    xdg_shell: String,
}

/// The wprsd command with the `-auth` shim first on its PATH.
fn wprsd_command(tools: &Tools, args: &[String], shim: PathBuf) -> Result<Command, LaunchError> {
    if let Some(wrapper) = &tools.wrapper {
        // The wrapper (for example `nix-portable nix shell … -c`) puts its
        // own directories first on PATH, so the shim goes first inside it.
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(format!(
                "exec {wrapper} sh -c 'PATH=\"$TERMINATOR_SHIM_DIR:$PATH\"; export PATH; exec \"$TERMINATOR_WPRSD\" \"$@\"' wprsd \"$@\""
            ))
            .arg("sh")
            .args(args)
            .env("TERMINATOR_SHIM_DIR", shim)
            .env("TERMINATOR_WPRSD", &tools.wprsd);
        return Ok(command);
    }
    let path = std::env::join_paths(std::iter::once(shim).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .map_err(io::Error::other)?;
    let mut command = Command::new(&tools.wprsd);
    command.args(args).env("PATH", path);
    Ok(command)
}

/// Shell words that start a command with Xwayland (and wprs, when it is
/// not next to the bridge) on its PATH, for example
/// `~/bin/nix-portable nix shell nixpkgs#xwayland -c`. wprsd then runs in
/// its own wrapper process, so it outlives the bridge.
pub const WRAPPER_ENV: &str = "TERMINATOR_WPRS_WRAPPER";

impl Tools {
    /// wprsd and xwayland-xdg-shell next to this binary (a Terminator
    /// install, so the same wprs as the bridge) come first. Else, with
    /// [`WRAPPER_ENV`], the wrapper's PATH provides them (and Xwayland);
    /// without it, PATH does.
    fn find() -> Result<Self, LaunchError> {
        let beside_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let beside = |name: &str| {
            beside_dir
                .as_ref()
                .map(|dir| dir.join(name))
                .filter(|path| path.is_file())
        };
        if let Some(wrapper) = std::env::var(WRAPPER_ENV)
            .ok()
            .filter(|w| !w.trim().is_empty())
        {
            return Ok(Self {
                wrapper: Some(wrapper),
                wprsd: beside("wprsd").unwrap_or_else(|| PathBuf::from("wprsd")),
                xdg_shell: beside("xwayland-xdg-shell").map_or_else(
                    || "xwayland-xdg-shell".to_owned(),
                    |path| path.display().to_string(),
                ),
            });
        }
        let find = |name: &'static str| {
            beside(name)
                .or_else(|| on_path(name))
                .ok_or(LaunchError::Missing(name))
        };
        on_path("Xwayland").ok_or(LaunchError::XwaylandMissing)?;
        Ok(Self {
            wrapper: None,
            wprsd: find("wprsd")?,
            xdg_shell: find("xwayland-xdg-shell")?.display().to_string(),
        })
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

fn free_display() -> Result<u32, LaunchError> {
    (FIRST_DISPLAY..=LAST_DISPLAY)
        .find(|n| {
            !Path::new(X11_DIR).join(format!("X{n}")).exists()
                && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
        })
        .ok_or(LaunchError::NoDisplay)
}

/// Headless hosts often have no X socket directory; smithay then fails with
/// a bare "No such file or directory".
fn ensure_x11_dir() -> Result<(), LaunchError> {
    if Path::new(X11_DIR).is_dir() {
        return Ok(());
    }
    match fs::create_dir(X11_DIR) {
        Ok(()) => fs::set_permissions(X11_DIR, fs::Permissions::from_mode(0o1777))?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// An Xauthority file with one MIT-MAGIC-COOKIE-1 for display `number`,
/// for this host name and for any host (FamilyWild), so clients match it
/// whatever address they use. Like vncserver: without it any local user
/// could connect, and under proot the server refuses local clients.
fn write_cookie(path: &Path, number: u32) -> Result<(), LaunchError> {
    let mut cookie = [0_u8; 16];
    File::open("/dev/urandom").and_then(|mut f| io::Read::read_exact(&mut f, &mut cookie))?;
    let host = nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    let number = number.to_string();
    let mut bytes = Vec::new();
    // 256 is FamilyLocal, 0xFFFF is FamilyWild.
    for (family, address) in [(256_u16, host.as_bytes()), (0xFFFF, &[][..])] {
        bytes.extend(family.to_be_bytes());
        for field in [address, number.as_bytes(), b"MIT-MAGIC-COOKIE-1", &cookie] {
            let len = u16::try_from(field.len()).map_err(io::Error::other)?;
            bytes.extend(len.to_be_bytes());
            bytes.extend(field);
        }
    }
    private_file(path)?.write_all(&bytes)?;
    Ok(())
}

/// xwayland-xdg-shell starts `Xwayland` from PATH with no way to add
/// arguments, so a shim first on PATH adds `-auth`.
/// The shim finds the real `Xwayland` on PATH when it runs (skipping its
/// own directory), so it also works inside a wrapper's environment.
fn xwayland_shim(dir: &Path, xauthority: &Path) -> Result<PathBuf, LaunchError> {
    let bin = dir.join("bin");
    private_dir(&bin)?;
    let script = format!(
        "#!/bin/sh\nself={}\nIFS=:\nfor d in $PATH; do\n  [ \"$d\" = \"$self\" ] && continue\n  [ -x \"$d/Xwayland\" ] && exec \"$d/Xwayland\" -auth {} \"$@\"\ndone\necho 'terminator-remote-bridge: Xwayland not found on PATH' >&2\nexit 127\n",
        shell_quote(&bin.to_string_lossy()),
        shell_quote(&xauthority.to_string_lossy()),
    );
    let shim = bin.join("Xwayland");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o700)
        .open(&shim)?;
    file.write_all(script.as_bytes())?;
    Ok(bin)
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Creates `dir` as 0700 unless it exists.
fn private_dir(dir: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

/// The last lines of a log, for error messages.
fn tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(8);
    lines.get(start..).unwrap_or_default().join("\n")
}
