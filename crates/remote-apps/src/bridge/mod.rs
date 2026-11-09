//! Host bridge (Linux only): a wprs client of the host's `wprsd` that speaks
//! the Terminator display protocol on stdin/stdout. The local daemon runs it
//! over an SSH exec channel, so the wprs protocol never leaves the host.
//!
//! Order of messages: `Hello` (versions), the windows that already exist,
//! `Ready` (the window list is complete), then changes. A window missing
//! after `Ready` is closed.
//!
//! Flow control: wprsd messages are taken as fast as they come and only
//! update [`state::State`]; frames are rendered when the writer thread has
//! sent the previous batch, so a slow link gets fewer, larger-damage frames
//! instead of a growing queue.
//!
//! When `wprsd` stops, the wprs client code ends this process with exit
//! status 1 and "server disconnected" on stderr; the daemon reports that as
//! [`ErrorKind::WprsdStopped`].

mod buffer;
mod clipboard;
mod input;
pub mod launch;
mod state;
mod takeover;

#[cfg(test)]
mod tests;

use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use wprs::serialization::{Event, RecvType, Request, SendType, Serializer};

use crate::protocol::{
    self, ClientMessage, ErrorKind, FrameError, Framed, HostMessage, PROTOCOL_VERSION,
};
use launch::{LaunchError, Session};

/// `Ready` goes out after this when wprsd never sends `ReplayDone` (a
/// wprsd started by an older bridge).
const REPLAY_LIMIT: Duration = Duration::from_secs(3);
/// How often a bridge checks that no newer bridge took over.
const OWNER_CHECK: Duration = Duration::from_secs(1);

/// What the bridge was started with.
pub struct Options {
    /// wprsd's socket. `None` finds or starts this user's wprsd
    /// (`--ensure`), and then `display` and `wayland_display` are ignored.
    pub socket: Option<PathBuf>,
    /// `DISPLAY` for X11 apps, if wprsd runs Xwayland.
    pub display: Option<String>,
    /// `WAYLAND_DISPLAY` for Wayland apps.
    pub wayland_display: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("cannot connect to wprsd at {path}: {reason}")]
    WprsdMissing { path: String, reason: String },
    #[error("{0}")]
    VersionMismatch(String),
    #[error("Another Terminator took these windows.")]
    TakenOver,
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("cannot write to the client: {0}")]
    Io(#[from] io::Error),
}

impl BridgeError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::WprsdMissing { .. } | Self::Launch(LaunchError::Missing(_)) => {
                ErrorKind::WprsdMissing
            }
            Self::Launch(LaunchError::XwaylandMissing) => ErrorKind::XwaylandMissing,
            Self::Launch(
                LaunchError::UnsafeDir(..)
                | LaunchError::NoDisplay
                | LaunchError::StartFailed { .. },
            ) => ErrorKind::HostSetup,
            Self::VersionMismatch(_) => ErrorKind::VersionMismatch,
            Self::TakenOver => ErrorKind::TakenOver,
            Self::Launch(LaunchError::StartTimeout { .. } | LaunchError::Io(_))
            | Self::Frame(_)
            | Self::Io(_) => ErrorKind::Other,
        }
    }
}

/// Where to connect, what to tell the client, and the private directory
/// when this bridge manages wprsd (`--ensure`).
struct Target {
    socket: PathBuf,
    display: Option<String>,
    wayland_display: String,
    managed: Option<(PathBuf, Session)>,
}

fn resolve(options: &Options) -> Result<Target, BridgeError> {
    if let Some(socket) = &options.socket {
        return Ok(Target {
            socket: socket.clone(),
            display: options.display.clone(),
            wayland_display: options.wayland_display.clone(),
            managed: None,
        });
    }
    let dir = launch::runtime_dir()?;
    // Before wprsd is used: an older bridge holds its only client slot.
    takeover::claim(&dir).map_err(LaunchError::Io)?;
    let session = match launch::live_session(&dir) {
        Some(session) => session,
        None => launch::start(&dir)?,
    };
    Ok(Target {
        socket: session.socket.clone(),
        display: Some(session.display.clone()),
        wayland_display: session.wayland_display.clone(),
        managed: Some((dir, session)),
    })
}

enum Incoming {
    Wprs(Box<RecvType<Request>>),
    Client(Box<Framed<ClientMessage>>),
    ClientGone,
    /// The writer sent a batch (or failed).
    Written(Result<(), String>),
}

/// The thread that owns stdout, so a slow link never stops the reading.
struct Writer {
    tx: mpsc::Sender<Vec<Framed<HostMessage>>>,
    thread: thread::JoinHandle<()>,
    /// A batch is on its way; frames wait until it is sent.
    busy: bool,
}

impl Writer {
    fn spawn(events: mpsc::Sender<Incoming>) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<Framed<HostMessage>>>();
        let thread = thread::spawn(move || {
            let mut out = BufWriter::new(io::stdout());
            while let Ok(batch) = rx.recv() {
                let result = batch
                    .iter()
                    .try_for_each(|f| protocol::write_frame(&mut out, &f.message, &f.payload))
                    .and_then(|()| out.flush().map_err(FrameError::from))
                    .map_err(|e| e.to_string());
                let failed = result.is_err();
                // The loop may be gone already (shutting down): keep writing.
                let _ = events.send(Incoming::Written(result));
                if failed {
                    return;
                }
            }
        });
        Self {
            tx,
            thread,
            busy: false,
        }
    }

    /// Writes every queued batch, then stops.
    fn finish(self) {
        drop(self.tx);
        let _ = self.thread.join();
    }

    fn send(&mut self, batch: Vec<Framed<HostMessage>>) {
        if !batch.is_empty() && self.tx.send(batch).is_ok() {
            self.busy = true;
        }
    }
}

/// Runs until the client closes stdin (Ok) or a fatal error. Errors are also
/// sent to the client as [`HostMessage::Error`] when stdout still works.
pub fn run(options: &Options) -> Result<(), BridgeError> {
    let (tx, rx) = mpsc::channel();
    let mut writer = Writer::spawn(tx.clone());
    let target = resolve(options);
    let (display, wayland_display) = match &target {
        Ok(t) => (t.display.clone(), t.wayland_display.clone()),
        Err(_) => (None, options.wayland_display.clone()),
    };
    writer.send(vec![Framed::new(HostMessage::Hello {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        display,
        wayland_display,
        capabilities: Vec::new(),
    })]);
    let result = target.and_then(|target| serve(target, &mut writer, tx, &rx));
    if let Err(error) = &result {
        writer.send(vec![Framed::new(HostMessage::Error {
            kind: error.kind(),
            message: error.to_string(),
        })]);
    }
    // The client must see the error before we exit.
    writer.finish();
    result
}

fn serve(
    target: Target,
    writer: &mut Writer,
    tx: mpsc::Sender<Incoming>,
    rx: &mpsc::Receiver<Incoming>,
) -> Result<(), BridgeError> {
    spawn_stdin(tx.clone());
    let serializer = connect_wprsd(&target.socket, tx)?;
    let wprs = serializer.writer();
    wprs.send(SendType::Object(Event::WprsClientConnect));
    wprs.send(SendType::Object(input::output()));

    let send = |event: Event| wprs.send(SendType::Object(event));
    let trace = std::env::var_os("TERMINATOR_BRIDGE_TRACE").is_some();
    let mut peers = Peers::default();
    // `Ready` is due when wprsd says its replay is done, or at this time
    // (a wprsd without `ReplayDone`). `None` once sent.
    let mut ready_by = Instant::now().checked_add(REPLAY_LIMIT);
    let mut owner_checked = Instant::now();
    loop {
        let wake = ready_by.map_or(OWNER_CHECK, |at| {
            at.saturating_duration_since(Instant::now())
                .min(OWNER_CHECK)
        });
        let incoming = match rx.recv_timeout(wake) {
            Ok(incoming) => Some(incoming),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        // Take everything that is waiting before rendering anything.
        for incoming in incoming.into_iter().chain(rx.try_iter()) {
            match incoming {
                Incoming::Wprs(request) => {
                    if trace {
                        trace_request(&request);
                    }
                    if matches!(*request, RecvType::Object(Request::ReplayDone)) {
                        ready_by = ready_by.map(|_| Instant::now());
                    }
                    peers.on_wprsd(*request, send);
                }
                Incoming::Client(framed) => peers.on_client(*framed, send)?,
                Incoming::ClientGone => return Ok(()),
                Incoming::Written(result) => {
                    writer.busy = false;
                    result.map_err(io::Error::other)?;
                }
            }
        }
        if let Some((dir, _)) = &target.managed
            && owner_checked.elapsed() >= OWNER_CHECK
        {
            owner_checked = Instant::now();
            if !takeover::still_owner(dir) {
                return Err(BridgeError::TakenOver);
            }
        }
        // Controls always go; frames only when the last batch is out.
        let mut batch = peers.state.flush(!writer.busy);
        if ready_by.is_some_and(|at| Instant::now() >= at) {
            ready_by = None;
            // After the batch that opens the replayed windows.
            batch.push(Framed::new(HostMessage::Ready));
            // wprsd has our output now: shells may use the display.
            if let Some((dir, session)) = &target.managed {
                launch::publish_for_shells(dir, session)?;
            }
        }
        writer.send(batch);
    }
}

/// Connects to wprsd at `socket` and forwards its requests to `tx` from a
/// reader thread.
fn connect_wprsd(
    socket: &std::path::Path,
    tx: mpsc::Sender<Incoming>,
) -> Result<Serializer<Event, Request>, BridgeError> {
    let mut serializer = Serializer::<Event, Request>::new_client(socket).map_err(|e| {
        BridgeError::WprsdMissing {
            path: socket.display().to_string(),
            reason: e.to_string(),
        }
    })?;
    if let Some(reader) = serializer.reader() {
        thread::spawn(move || {
            while let Ok(request) = reader.recv() {
                if tx.send(Incoming::Wprs(Box::new(request))).is_err() {
                    break;
                }
            }
        });
    }
    Ok(serializer)
}

/// What `serve` keeps between messages: the windows, input and clipboard
/// state between wprsd and the client.
#[derive(Default)]
struct Peers {
    state: state::State,
    input: input::Input,
    clipboard: clipboard::Clipboard,
}

impl Peers {
    fn on_wprsd(&mut self, request: RecvType<Request>, send: impl Fn(Event)) {
        match request {
            RecvType::Object(Request::Data(data)) => {
                let (events, frames) = self.clipboard.on_wprsd(data);
                events.into_iter().for_each(send);
                for frame in frames {
                    self.state.queue(frame);
                }
            }
            request => self.state.handle(request),
        }
    }

    fn on_client(
        &mut self,
        framed: Framed<ClientMessage>,
        send: impl Fn(Event),
    ) -> Result<(), BridgeError> {
        let Framed { message, payload } = framed;
        if let ClientMessage::Hello { protocol, .. } = message
            && protocol != PROTOCOL_VERSION
        {
            return Err(BridgeError::VersionMismatch(
                protocol::version_mismatch_message(
                    PROTOCOL_VERSION,
                    env!("CARGO_PKG_VERSION"),
                    protocol,
                ),
            ));
        }
        let mut events = self.clipboard.on_client(&message, payload);
        let state = &self.state;
        events.extend(
            self.input
                .events(&message, |id| state.surface_of(id).map(|key| key.1)),
        );
        events.into_iter().for_each(send);
        Ok(())
    }
}

/// Reads client frames from stdin until it closes or sends garbage.
fn spawn_stdin(tx: mpsc::Sender<Incoming>) {
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        loop {
            let incoming = match protocol::read_frame::<ClientMessage>(&mut input) {
                Ok(framed) => Incoming::Client(Box::new(framed)),
                Err(_) => Incoming::ClientGone,
            };
            let gone = matches!(incoming, Incoming::ClientGone);
            if tx.send(incoming).is_err() || gone {
                break;
            }
        }
    });
}

/// One stderr line per wprsd request, without pixels, for debugging.
#[expect(
    clippy::print_stderr,
    reason = "Opt-in developer trace; stdout carries the protocol."
)]
fn trace_request(request: &RecvType<Request>) {
    match request {
        RecvType::RawBuffer(bytes) => eprintln!("wprs: raw buffer {} bytes", bytes.len()),
        RecvType::Object(object) => {
            let text = format!("{object:?}");
            let short: String = text.chars().take(300).collect();
            eprintln!("wprs: {short}");
        }
    }
}
