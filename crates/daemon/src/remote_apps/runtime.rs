//! One remote-apps session's live state, and how bridge messages change it.
//!
//! The session is a small state machine ([`Phase`]):
//!
//! ```text
//! Running{ready: false} --Ready--> Running{ready: true}
//!        |  bridge exits (temporary failure)        |
//!        v                                           v
//!     Waiting{attempt, delay} --retry_started--> Running{ready: false}
//!        |  permanent failure, or stop requested
//!        v
//!     Ended(Failed | Disconnected)
//! ```
//!
//! The status the GUI sees comes only from the phase: `connected` means the
//! bridge sent its whole window list (`Ready`), so the GUI may close a tab
//! whose window is missing. An `Error` from the bridge is kept apart until
//! the bridge exits; only then does the worker decide between a retry and
//! the end.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use terminator_core::remote_apps::protocol::Rect;
use terminator_core::remote_apps::{
    ClientMessage, ErrorKind, HostMessage, MAX_CLIPBOARD, RemoteAppsState, RemoteAppsStatus,
    RemoteClipboard, RemoteCursor, RemoteWindow, WindowId, protocol, retry_delay_seconds,
};

use super::pixels;
use super::window::Window;

/// One message to the bridge and its payload (clipboard bytes).
pub(crate) type Input = (ClientMessage, Vec<u8>);
/// A failure the GUI shows.
pub(crate) type Failure = (ErrorKind, String);

const FILE_PREFIX: &str = "remote-app-";
const FILE_SUFFIX: &str = ".fb";

pub(crate) enum Phase {
    /// A bridge runs (or is starting). `ready`: it sent the whole window
    /// list. `input` is `None` until its stdin is attached.
    Running {
        input: Option<mpsc::Sender<Input>>,
        ready: bool,
        attempt: u32,
    },
    /// The bridge stopped; retry `attempt` starts after `delay` seconds.
    Waiting {
        attempt: u32,
        delay: u64,
    },
    Ended(RemoteAppsStatus),
}

/// What the worker does after a bridge exited.
pub(crate) enum Next {
    Retry(u64),
    Stop,
}

pub(crate) struct Runtime {
    pub(crate) session: String,
    pub(crate) project_id: String,
    pub(crate) phase: Phase,
    /// The last failure, shown while retrying and after the end.
    failure: Option<Failure>,
    /// An `Error` the running bridge sent; it wins over the exit reason.
    bridge_error: Option<Failure>,
    display: Option<String>,
    pub(crate) revision: u64,
    windows: BTreeMap<WindowId, Window>,
    /// The user (or the detach timeout) asked to stop; no more retries.
    pub(crate) stop_requested: bool,
    /// Last time a viewer asked for the state, for the detach timeout.
    pub(crate) last_viewed: Instant,
    shm_dir: PathBuf,
    files: u64,
    /// Its `serial` is the revision that set it.
    cursor: Option<RemoteCursor>,
    /// Its `serial` is the revision that set it.
    clipboard: Option<RemoteClipboard>,
}

impl Runtime {
    pub(crate) fn new(session: String, project_id: String, shm_dir: PathBuf) -> Self {
        Self {
            session,
            project_id,
            phase: Phase::Running {
                input: None,
                ready: false,
                attempt: 0,
            },
            failure: None,
            bridge_error: None,
            display: None,
            revision: 1,
            windows: BTreeMap::new(),
            stop_requested: false,
            last_viewed: Instant::now(),
            shm_dir,
            files: 0,
            cursor: None,
            clipboard: None,
        }
    }

    pub(crate) fn status(&self) -> RemoteAppsStatus {
        match self.phase {
            Phase::Running { ready: true, .. } => RemoteAppsStatus::Connected,
            Phase::Running { .. } | Phase::Waiting { .. } => RemoteAppsStatus::Connecting,
            Phase::Ended(status) => status,
        }
    }

    pub(crate) fn input(&self) -> Option<&mpsc::Sender<Input>> {
        match &self.phase {
            Phase::Running { input, .. } => input.as_ref(),
            _ => None,
        }
    }

    /// Whether this is a live session of `project`.
    pub(crate) fn live_for(&self, project: &str) -> bool {
        self.project_id == project && self.status().live()
    }

    /// A new bridge runs; `tx` feeds its stdin. `false` when a stop came
    /// first (the caller closes the bridge's stdin at once).
    pub(crate) fn bridge_started(&mut self, tx: mpsc::Sender<Input>) -> bool {
        if self.stop_requested {
            return false;
        }
        if let Phase::Running { input, .. } = &mut self.phase {
            *input = Some(tx);
        }
        self.bridge_error = None;
        true
    }

    /// The bridge exited with `exit` (classified from its status and
    /// stderr). Decides between a retry and the end.
    pub(crate) fn bridge_exited(&mut self, exit: Failure) -> Next {
        let attempt = match self.phase {
            Phase::Running { attempt, .. } => attempt,
            _ => 0,
        };
        let failure = self.bridge_error.take().unwrap_or(exit);
        if self.stop_requested || failure.0.permanent() {
            self.end(Some(failure));
            return Next::Stop;
        }
        self.windows.clear();
        let attempt = attempt.saturating_add(1);
        let delay = retry_delay_seconds(attempt);
        self.phase = Phase::Waiting { attempt, delay };
        self.failure = Some(failure);
        self.bump();
        Next::Retry(delay)
    }

    /// Ends the session: `Disconnected` when the user asked to stop (the
    /// failure is then not shown) or there is no failure, else `Failed`.
    pub(crate) fn end(&mut self, failure: Option<Failure>) {
        self.windows.clear();
        self.failure = failure.filter(|_| !self.stop_requested);
        self.phase = Phase::Ended(if self.failure.is_some() {
            RemoteAppsStatus::Failed
        } else {
            RemoteAppsStatus::Disconnected
        });
        self.bump();
    }

    /// A retry starts now.
    pub(crate) fn retry_started(&mut self) {
        let attempt = match self.phase {
            Phase::Waiting { attempt, .. } => attempt,
            _ => 0,
        };
        self.phase = Phase::Running {
            input: None,
            ready: false,
            attempt,
        };
        self.bump();
    }

    /// Stops the session: no more retries, and the bridge's stdin closes
    /// (it exits; the apps stay in wprsd).
    pub(crate) fn request_stop(&mut self) {
        self.stop_requested = true;
        self.close_input();
    }

    /// Closes the running bridge's stdin, so it exits.
    fn close_input(&mut self) {
        if let Phase::Running { input, .. } = &mut self.phase {
            *input = None;
        }
    }

    /// The state for a viewer that already has everything up to `since`.
    pub(crate) fn snapshot(&self, since: u64) -> RemoteAppsState {
        let mut windows: Vec<RemoteWindow> = self
            .windows
            .values()
            .map(|w| {
                let mut meta = w.meta.clone();
                meta.regions = w.changed_since(since).into_iter().collect();
                meta
            })
            .collect();
        // Parents before popups, so the GUI can place each popup on arrival.
        windows.sort_by_key(|w| (matches!(w.role, protocol::WindowRole::Popup { .. }), w.id));
        let (retry_attempt, retry_in_seconds) = match self.phase {
            Phase::Waiting { attempt, delay } => (attempt, delay),
            Phase::Running { attempt, .. } => (attempt, 0),
            Phase::Ended(_) => (0, 0),
        };
        RemoteAppsState {
            session: self.session.clone(),
            project_id: self.project_id.clone(),
            status: self.status(),
            error: self.failure.as_ref().map(|f| f.1.clone()),
            error_kind: self.failure.as_ref().map(|f| f.0),
            display: self.display.clone(),
            revision: self.revision,
            retry_attempt,
            retry_in_seconds,
            windows,
            cursor: self.cursor.clone().filter(|c| c.serial > since),
            clipboard: self.clipboard.clone().filter(|c| c.serial > since),
        }
    }

    fn bump(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    /// Applies one bridge message; `rgba` is a `Frame`'s rectangles from
    /// [`pixels::decode_frame`]. Returns whether anything changed.
    pub(crate) fn apply(
        &mut self,
        framed: protocol::Framed<HostMessage>,
        rgba: Option<Vec<Vec<u8>>>,
    ) -> bool {
        match framed.message {
            HostMessage::Hello {
                protocol: version,
                version: host_version,
                display,
                ..
            } => self.hello(version, &host_version, display),
            HostMessage::Ready => {
                if let Phase::Running { ready, attempt, .. } = &mut self.phase {
                    *ready = true;
                    *attempt = 0;
                }
                self.failure = None;
            }
            HostMessage::WindowOpened {
                id,
                role,
                title,
                app_id,
            } => self.open_window(id, role, (title, app_id)),
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
                id,
                width,
                height,
                damage,
                ..
            } => return self.frame(id, (width, height), (&damage, rgba)),
            HostMessage::WindowClosed { id } => {
                if self.windows.remove(&id).is_none() {
                    return false;
                }
            }
            HostMessage::Error { kind, message } => self.bridge_error = Some((kind, message)),
            HostMessage::Cursor {
                width,
                height,
                hotspot_x,
                hotspot_y,
                name,
            } => {
                let cursor = pixels::cursor(
                    framed.payload,
                    (width, height),
                    (hotspot_x, hotspot_y),
                    name,
                );
                return self.set_cursor(cursor);
            }
            HostMessage::ClipboardData { .. } => return self.copied(&framed.payload),
        }
        self.bump();
        true
    }

    fn hello(&mut self, version: u32, host_version: &str, display: Option<String>) {
        if version != protocol::PROTOCOL_VERSION {
            self.bridge_error = Some((
                ErrorKind::VersionMismatch,
                protocol::version_mismatch_message(
                    version,
                    host_version,
                    protocol::PROTOCOL_VERSION,
                ),
            ));
            // Nothing more from this bridge makes sense: stop it.
            self.close_input();
        }
        self.display = display;
    }

    fn open_window(
        &mut self,
        id: WindowId,
        role: protocol::WindowRole,
        (title, app_id): (String, String),
    ) {
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
            regions: Vec::new(),
        };
        self.windows.insert(id, Window::new(meta));
    }

    fn set_cursor(&mut self, cursor: Option<RemoteCursor>) -> bool {
        let Some(mut cursor) = cursor else {
            return false;
        };
        cursor.serial = self.bump();
        self.cursor = Some(cursor);
        true
    }

    /// Text a remote app copied.
    fn copied(&mut self, text: &[u8]) -> bool {
        if text.len() > MAX_CLIPBOARD {
            return false;
        }
        self.clipboard = Some(RemoteClipboard {
            serial: self.bump(),
            text: String::from_utf8_lossy(text).into_owned(),
        });
        true
    }

    /// Writes the damage into the window's shared-memory file; a new size gets a new file and must come as
    /// one whole-window rectangle. Bad frames are dropped.
    fn frame(
        &mut self,
        id: WindowId,
        size: (u32, u32),
        (damage, rgba): (&[Rect], Option<Vec<Vec<u8>>>),
    ) -> bool {
        let (Ok(width), Ok(height), Some(rgba)) =
            (u16::try_from(size.0), u16::try_from(size.1), rgba)
        else {
            return false;
        };
        let Some(old_size) = self.windows.get(&id).map(Window::size) else {
            return false;
        };
        let resized = old_size != Some((width, height));
        let whole = Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        };
        if resized && damage != [whole] {
            return false;
        }
        let path = resized.then(|| self.next_path());
        let revision = self.revision.saturating_add(1);
        let published = self
            .windows
            .get_mut(&id)
            .is_some_and(|window| window.publish((revision, (width, height), path), damage, &rgba));
        if published {
            self.revision = revision;
        }
        published
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
