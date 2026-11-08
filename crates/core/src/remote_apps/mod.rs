//! Remote app windows (`remote-apps-v1`): the daemon runs the host bridge
//! (`terminator-remote-bridge --ensure`) over the SSH master of a remote
//! project and keeps each window's pixels in a shared-memory file. The GUI
//! long-polls [`RemoteAppsState`] and sends input. See docs/REMOTE_APPS.md.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
pub use terminator_remote_apps::compose::zstd_bgra_to_rgba;
pub use terminator_remote_apps::protocol::{
    self, ClientMessage, ErrorKind, HostMessage, PointerButton, WindowId, WindowRole,
};

pub const CAPABILITY: &str = "remote-apps-v1";
/// Host command when the project sets none: a Terminator install on the
/// host puts the bridge on `PATH`.
pub const DEFAULT_COMMAND: &str = "terminator-remote-bridge --ensure";
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
    /// All open windows, toplevels before their popups.
    pub windows: Vec<RemoteWindow>,
}
