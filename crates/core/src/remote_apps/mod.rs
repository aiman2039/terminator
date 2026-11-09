//! Remote app windows (`remote-apps-v1`): the daemon runs the host bridge
//! (`terminator-remote-bridge --ensure`) over the SSH master of a remote
//! project and keeps each window's pixels in a shared-memory file. The GUI
//! long-polls [`RemoteAppsState`] and sends input. See docs/REMOTE_APPS.md.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
pub use terminator_remote_apps::compose::{union, zstd_bgra_to_rgba};
pub use terminator_remote_apps::protocol::{
    self, ClientMessage, ErrorKind, HostMessage, PointerButton, WindowId, WindowRole,
};

pub const CAPABILITY: &str = "remote-apps-v1";
/// Host command when the project sets none: a Terminator install on the
/// host puts the bridge on `PATH`.
pub const DEFAULT_COMMAND: &str = "terminator-remote-bridge --ensure";
/// Shell code (POSIX `sh`, safe under `set -eu`) that new SSH terminals run
/// first: when this user's bridge recorded a session (`session.json` in
/// `/tmp/terminator-remote-<uid>`, or `$TERMINATOR_REMOTE_DIR`; see the
/// bridge's `--ensure`), its wprsd still runs and its X socket exists, it
/// exports `DISPLAY`, `XAUTHORITY` and `WAYLAND_DISPLAY`, so apps started in
/// the terminal open as Terminator tabs. A dead session prints one line and
/// exports nothing.
pub const SHELL_ENV: &str = r#"__t="${TERMINATOR_REMOTE_DIR:-/tmp/terminator-remote-$(id -u)}"; if [ -f "$__t/session.json" ]; then __d=$(sed -n 's/.*"display": *"\(:[0-9]*\)".*/\1/p' "$__t/session.json"); __p=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$__t/session.json"); if [ -n "$__d" ] && [ -n "$__p" ] && kill -0 "$__p" 2>/dev/null && [ -S "/tmp/.X11-unix/X${__d#:}" ]; then export DISPLAY="$__d" XAUTHORITY="$__t/Xauthority" WAYLAND_DISPLAY="$__t/wprs-0"; else echo "terminator: remote apps are not running on this host (use Show remote apps)" >&2; fi; fi; unset __t __d __p; "#;
/// The daemon's error for a session it does not know (it restarted). Every
/// daemon version sends this text; the GUI connects again when it sees it.
pub const UNKNOWN_SESSION: &str = "Unknown remote-apps session";
/// Overrides the bridge's private directory (tests).
pub const REMOTE_DIR_ENV: &str = "TERMINATOR_REMOTE_DIR";
/// Largest clipboard text carried either way.
pub const MAX_CLIPBOARD: usize = 1024 * 1024;
pub use terminator_remote_apps::protocol::TEXT_MIME_TYPES;
/// Most input messages in one request.
pub const MAX_INPUT: usize = 64;
/// Longest a state request waits for a newer revision.
pub const MAX_WAIT_MS: u32 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAppsStatus {
    Connecting,
    Connected,
    /// The bridge stopped; `error` says why. Apps on the host may still run
    /// (wprsd keeps them), so connecting again reattaches.
    Failed,
    Disconnected,
}

impl RemoteAppsStatus {
    #[must_use]
    pub fn live(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }
}

/// One remote window. Pixels are premultiplied RGBA in `shm` (the
/// `terminator_sys` shared frame format), consistent at `sequence`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteWindow {
    pub id: WindowId,
    pub role: WindowRole,
    pub title: String,
    pub app_id: String,
    pub width: u16,
    pub height: u16,
    /// Session revision of the last change to this window.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shm: Option<PathBuf>,
    #[serde(default)]
    pub sequence: u64,
    /// The only areas that changed after the request's `since` revision.
    /// Empty means "read the whole window" (new, resized, or too old).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<protocol::Rect>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteAppsState {
    pub session: String,
    pub project_id: String,
    pub status: RemoteAppsStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    /// `DISPLAY` on the host for X11 apps, once the bridge said hello.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Grows on every change; the GUI sends the last one it saw as `since`.
    pub revision: u64,
    /// While `Connecting` after a failure: which retry this is, and seconds
    /// until it starts. `error` keeps the failure. Zero otherwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retry_attempt: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub retry_in_seconds: u64,
    /// All open windows, toplevels before their popups.
    pub windows: Vec<RemoteWindow>,
    /// The pointer shape the apps ask for; sent only when it changed after
    /// the request's `since`, so the GUI keeps the last one it got.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<RemoteCursor>,
    /// The last text a remote app copied, sent only when it changed after
    /// `since`. The GUI puts it on the local clipboard only while one of
    /// this host's windows has focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<RemoteClipboard>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteClipboard {
    /// Changes with every copy.
    pub serial: u64,
    pub text: String,
}

/// A remote cursor: a named shape (`name`, a CSS cursor name), an image
/// (`rgba`, base64 premultiplied RGBA of `width` x `height`), or hidden
/// (neither).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCursor {
    /// Changes with every new cursor, so the GUI rebuilds its image once.
    pub serial: u64,
    pub width: u16,
    pub height: u16,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rgba: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// A protocol rectangle as a shared-frame rectangle; `None` when it does
/// not fit in `u16` coordinates.
#[must_use]
pub fn frame_rect(rect: protocol::Rect) -> Option<terminator_sys::FrameRect> {
    Some(terminator_sys::FrameRect {
        x: u16::try_from(rect.x).ok()?,
        y: u16::try_from(rect.y).ok()?,
        width: u16::try_from(rect.width).ok()?,
        height: u16::try_from(rect.height).ok()?,
    })
}

/// Seconds before retry `attempt` (1-based): 1, 2, 4, … capped at 60.
#[must_use]
pub fn retry_delay_seconds(attempt: u32) -> u64 {
    1_u64
        .checked_shl(attempt.saturating_sub(1).min(6))
        .unwrap_or(60)
        .min(60)
}

#[cfg(test)]
mod tests {
    use super::SHELL_ENV;
    use std::os::unix::net::UnixListener;
    use std::process::Command;

    /// Runs the snippet like a remote terminal does; returns stdout, stderr.
    fn run(runtime: &std::path::Path) -> (String, String) {
        let script = format!(
            "set -eu; {SHELL_ENV} echo \"D=${{DISPLAY:-none}} X=${{XAUTHORITY:-none}} W=${{WAYLAND_DISPLAY:-none}}\""
        );
        let out = Command::new("sh")
            .arg("-c")
            .arg(script)
            .env(super::REMOTE_DIR_ENV, runtime.join("terminator-remote"))
            .env_remove("DISPLAY")
            .env_remove("XAUTHORITY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        (
            String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )
    }

    #[test]
    fn no_session_exports_nothing_and_says_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (out, err) = run(dir.path());
        assert_eq!(out, "D=none X=none W=none");
        assert!(err.is_empty() || err.contains("remote apps"), "{err}");
    }

    #[test]
    fn dead_session_says_so_and_exports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("terminator-remote");
        std::fs::create_dir(&session).unwrap();
        // A wprsd that is gone: no such process and no socket.
        std::fs::write(
            session.join("session.json"),
            r#"{"display": ":4999", "pid": 999999999}"#,
        )
        .unwrap();
        let (out, err) = run(dir.path());
        assert_eq!(out, "D=none X=none W=none");
        assert!(err.contains("remote apps are not running"), "{err}");
    }

    #[test]
    fn live_session_exports_display_cookie_and_wayland_socket() {
        // Needs a writable /tmp/.X11-unix, as on the hosts this runs on.
        let x11 = std::path::Path::new("/tmp/.X11-unix");
        if std::fs::create_dir_all(x11).is_err() {
            return;
        }
        let number = 4000 + std::process::id() % 900;
        let socket = x11.join(format!("X{number}"));
        let Ok(_listener) = UnixListener::bind(&socket) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("terminator-remote");
        std::fs::create_dir(&session).unwrap();
        std::fs::write(
            session.join("session.json"),
            format!(
                "{{\n  \"display\": \":{number}\",\n  \"pid\": {}\n}}",
                std::process::id()
            ),
        )
        .unwrap();
        let (out, _) = run(dir.path());
        let _ = std::fs::remove_file(&socket);
        let t = session.display();
        assert_eq!(out, format!("D=:{number} X={t}/Xauthority W={t}/wprs-0"));
    }
}
