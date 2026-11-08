//! One remote-apps session's live state, and how bridge messages change it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use base64::Engine as _;
use terminator_core::remote_apps::{
    ClientMessage, ErrorKind, HostMessage, MAX_CLIPBOARD, RemoteAppsState, RemoteAppsStatus,
    RemoteClipboard, RemoteCursor, RemoteWindow, WindowId, protocol,
};
use terminator_sys::SharedFrameWriter;

/// One message to the bridge and its payload (clipboard bytes).
pub(crate) type Input = (ClientMessage, Vec<u8>);

const FILE_PREFIX: &str = "remote-app-";
const FILE_SUFFIX: &str = ".fb";

struct Window {
    meta: RemoteWindow,
    /// Dropping it removes its file.
    writer: Option<SharedFrameWriter>,
}

pub(crate) struct Runtime {
    pub(crate) session: String,
    pub(crate) project_id: String,
    pub(crate) status: RemoteAppsStatus,
    error: Option<String>,
    error_kind: Option<ErrorKind>,
    display: Option<String>,
    pub(crate) revision: u64,
    windows: BTreeMap<WindowId, Window>,
    /// Input to the bridge; `None` while no bridge runs.
    pub(crate) input: Option<mpsc::Sender<Input>>,
    /// The user (or the detach timeout) asked to stop; no more retries.
    pub(crate) stop_requested: bool,
    /// Last time a viewer asked for the state, for the detach timeout.
    pub(crate) last_viewed: Instant,
    /// The running bridge said hello (it reached wprsd).
    pub(crate) hello_seen: bool,
    retry_attempt: u32,
    retry_in_seconds: u64,
    shm_dir: PathBuf,
    files: u64,
    cursor: Option<RemoteCursor>,
    clipboard: Option<RemoteClipboard>,
}

impl Runtime {
    pub(crate) fn new(session: String, project_id: String, shm_dir: PathBuf) -> Self {
        Self {
            session,
            project_id,
            status: RemoteAppsStatus::Connecting,
            error: None,
            error_kind: None,
            display: None,
            revision: 1,
            windows: BTreeMap::new(),
            input: None,
            stop_requested: false,
            last_viewed: Instant::now(),
            hello_seen: false,
            retry_attempt: 0,
            retry_in_seconds: 0,
            shm_dir,
            files: 0,
            cursor: None,
            clipboard: None,
        }
    }

    /// The error the bridge itself sent before it stopped, if any.
    pub(crate) fn bridge_error(&self) -> Option<(ErrorKind, String)> {
        (self.status == RemoteAppsStatus::Failed)
            .then(|| self.error_kind.zip(self.error.clone()))
            .flatten()
    }

    /// The bridge stopped and a retry follows in `delay` seconds. Windows
    /// close now; the next bridge reopens them with the same IDs.
    pub(crate) fn retrying(&mut self, failure: (ErrorKind, String), attempt: u32, delay: u64) {
        self.input = None;
        self.hello_seen = false;
        self.windows.clear();
        self.status = RemoteAppsStatus::Connecting;
        self.error_kind = Some(failure.0);
        self.error = Some(failure.1);
        self.retry_attempt = attempt;
        self.retry_in_seconds = delay;
        self.bump();
    }

    /// A retry starts now.
    pub(crate) fn retry_started(&mut self) {
        self.retry_in_seconds = 0;
        self.bump();
    }

    pub(crate) fn snapshot(&self) -> RemoteAppsState {
        let mut windows: Vec<RemoteWindow> =
            self.windows.values().map(|w| w.meta.clone()).collect();
        // Parents before popups, so the GUI can place each popup on arrival.
        windows.sort_by_key(|w| (matches!(w.role, protocol::WindowRole::Popup { .. }), w.id));
        RemoteAppsState {
            session: self.session.clone(),
            project_id: self.project_id.clone(),
            status: self.status,
            error: self.error.clone(),
            error_kind: self.error_kind,
            display: self.display.clone(),
            revision: self.revision,
            retry_attempt: self.retry_attempt,
            retry_in_seconds: self.retry_in_seconds,
            windows,
            cursor: self.cursor.clone(),
            clipboard: self.clipboard.clone(),
        }
    }

    fn bump(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    /// Ends the session: windows close and their files go away.
    pub(crate) fn end(&mut self, status: RemoteAppsStatus, error: Option<(ErrorKind, String)>) {
        self.input = None;
        self.windows.clear();
        self.retry_in_seconds = 0;
        if self.status == RemoteAppsStatus::Failed && self.error.is_some() {
            // Keep the bridge's own explanation (it sent Error, then exited).
        } else {
            self.status = status;
            self.error_kind = error.as_ref().map(|e| e.0);
            self.error = error.map(|e| e.1);
        }
        self.bump();
    }

    /// Applies one bridge message. Returns whether anything changed.
    pub(crate) fn apply(&mut self, framed: protocol::Framed<HostMessage>) -> bool {
        match framed.message {
            HostMessage::Hello {
                protocol: version,
                version: host_version,
                display,
                ..
            } => {
                if version == protocol::PROTOCOL_VERSION {
                    self.status = RemoteAppsStatus::Connected;
                    self.hello_seen = true;
                    self.error = None;
                    self.error_kind = None;
                    self.retry_attempt = 0;
                    self.retry_in_seconds = 0;
                    self.display = display;
                } else {
                    self.fail(
                        ErrorKind::VersionMismatch,
                        protocol::version_mismatch_message(
                            version,
                            &host_version,
                            protocol::PROTOCOL_VERSION,
                        ),
                    );
                }
            }
            HostMessage::WindowOpened {
                id,
                role,
                title,
                app_id,
            } => {
                let revision = self.bump();
                let meta = RemoteWindow {
                    id,
                    role,
                    title,
                    app_id,
                    width: 0,
                    height: 0,
                    revision,
                    shm: None,
                    sequence: 0,
                };
                self.windows.insert(id, Window { meta, writer: None });
            }
            HostMessage::WindowChanged { id, title, app_id } => {
                let revision = self.bump();
                let Some(window) = self.windows.get_mut(&id) else {
                    return false;
                };
                window.meta.title = title;
                window.meta.app_id = app_id;
                window.meta.revision = revision;
            }
            HostMessage::Frame {
                id, width, height, ..
            } => return self.frame(id, (width, height), &framed.payload),
            HostMessage::WindowClosed { id } => {
                if self.windows.remove(&id).is_none() {
                    return false;
                }
            }
            HostMessage::Error { kind, message } => self.fail(kind, message),
            HostMessage::Cursor {
                width,
                height,
                hotspot_x,
                hotspot_y,
                name,
            } => {
                let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
                    return false;
                };
                let mut rgba = framed.payload;
                let expected = usize::from(width)
                    .saturating_mul(usize::from(height))
                    .saturating_mul(4);
                if rgba.len() != expected {
                    rgba.clear();
                }
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let serial = self
                    .cursor
                    .as_ref()
                    .map_or(1, |c| c.serial.saturating_add(1));
                self.cursor = Some(RemoteCursor {
                    serial,
                    width,
                    height,
                    hotspot_x,
                    hotspot_y,
                    name,
                    rgba: base64::engine::general_purpose::STANDARD.encode(rgba),
                });
            }
            HostMessage::ClipboardData { .. } => {
                if framed.payload.len() > MAX_CLIPBOARD {
                    return false;
                }
                let serial = self
                    .clipboard
                    .as_ref()
                    .map_or(1, |c| c.serial.saturating_add(1));
                self.clipboard = Some(RemoteClipboard {
                    serial,
                    text: String::from_utf8_lossy(&framed.payload).into_owned(),
                });
            }
            HostMessage::ClipboardOffer { .. } => return false,
        }
        self.bump();
        true
    }

    fn fail(&mut self, kind: ErrorKind, message: String) {
        self.status = RemoteAppsStatus::Failed;
        self.error_kind = Some(kind);
        self.error = Some(message);
    }

    /// Publishes a frame into the window's shared-memory file, making a new
    /// file when the size changed. Bad frames are dropped.
    fn frame(&mut self, id: WindowId, size: (u32, u32), payload: &[u8]) -> bool {
        let (Ok(width), Ok(height)) = (u16::try_from(size.0), u16::try_from(size.1)) else {
            return false;
        };
        let Some(rgba) = terminator_core::remote_apps::zstd_bgra_to_rgba(payload, size.0, size.1)
        else {
            return false;
        };
        let path = self.next_path();
        let revision = self.revision.saturating_add(1);
        let Some(window) = self.windows.get_mut(&id) else {
            return false;
        };
        if window.writer.as_ref().map(SharedFrameWriter::size) != Some((width, height)) {
            window.writer = None;
            match SharedFrameWriter::create(&path, width, height) {
                Ok(writer) => window.writer = Some(writer),
                Err(_) => return false,
            }
        }
        let Some(writer) = window.writer.as_mut() else {
            return false;
        };
        let Ok(sequence) = writer.publish(&rgba, &[]) else {
            return false;
        };
        window.meta.width = width;
        window.meta.height = height;
        window.meta.shm = Some(writer.path().to_path_buf());
        window.meta.sequence = sequence;
        window.meta.revision = revision;
        self.revision = revision;
        true
    }

    fn next_path(&mut self) -> PathBuf {
        self.files = self.files.saturating_add(1);
        self.shm_dir.join(format!(
            "{FILE_PREFIX}{}-{}-{}{FILE_SUFFIX}",
            std::process::id(),
            self.session,
            self.files
        ))
    }
}
