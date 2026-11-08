//! Host bridge (Linux only): a wprs client of the host's `wprsd` that speaks
//! the Terminator display protocol on stdin/stdout. The local daemon runs it
//! over an SSH exec channel, so the wprs protocol never leaves the host.
//!
//! When `wprsd` stops, the wprs client code ends this process with exit
//! status 1 and "server disconnected" on stderr; the daemon reports that as
//! [`ErrorKind::WprsdStopped`].

mod input;
mod state;

#[cfg(test)]
mod tests;

use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use wprs::serialization::{Event, RecvType, Request, SendType, Serializer};

use crate::protocol::{
    self, ClientMessage, ErrorKind, FrameError, Framed, HostMessage, PROTOCOL_VERSION,
};

/// What the bridge was started with.
pub struct Options<'a> {
    /// wprsd's socket.
    pub socket: &'a Path,
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
    #[error(transparent)]
    Frame(#[from] FrameError),
}

enum Incoming {
    Wprs(Box<RecvType<Request>>),
    Client(ClientMessage),
    ClientGone,
}

/// Runs until the client closes stdin (Ok) or a fatal error. Errors are also
/// sent to the client as [`HostMessage::Error`] when stdout still works.
pub fn run(options: &Options<'_>) -> Result<(), BridgeError> {
    let mut out = BufWriter::new(io::stdout().lock());
    send(
        &mut out,
        HostMessage::Hello {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            display: options.display.clone(),
            wayland_display: options.wayland_display.clone(),
            capabilities: Vec::new(),
        },
    )?;
    let result = serve(options, &mut out);
    if let Err(error) = &result {
        let kind = match error {
            BridgeError::WprsdMissing { .. } => ErrorKind::WprsdMissing,
            BridgeError::VersionMismatch(_) => ErrorKind::VersionMismatch,
            BridgeError::Frame(_) => ErrorKind::Other,
        };
        let message = error.to_string();
        // stdout may be the thing that failed; nothing more to do then.
        let _ = send(&mut out, HostMessage::Error { kind, message });
    }
    result
}

fn serve(options: &Options<'_>, out: &mut impl Write) -> Result<(), BridgeError> {
    let (tx, rx) = mpsc::channel();
    spawn_stdin(tx.clone());
    let mut serializer = Serializer::<Event, Request>::new_client(options.socket).map_err(|e| {
        BridgeError::WprsdMissing {
            path: options.socket.display().to_string(),
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
    let writer = serializer.writer();
    writer.send(SendType::Object(Event::WprsClientConnect));
    writer.send(SendType::Object(input::output()));

    let trace = std::env::var_os("TERMINATOR_BRIDGE_TRACE").is_some();
    let mut state = state::State::default();
    let mut input = input::Input::default();
    for incoming in rx {
        match incoming {
            Incoming::Wprs(request) => {
                if trace {
                    trace_request(&request);
                }
                for framed in state.handle(*request) {
                    protocol::write_frame(out, &framed.message, &framed.payload)?;
                }
            }
            Incoming::Client(ClientMessage::Hello { protocol, .. })
                if protocol != PROTOCOL_VERSION =>
            {
                return Err(BridgeError::VersionMismatch(
                    protocol::version_mismatch_message(
                        PROTOCOL_VERSION,
                        env!("CARGO_PKG_VERSION"),
                        protocol,
                    ),
                ));
            }
            Incoming::Client(message) => {
                let events = input.events(&message, |id| state.surface_of(id).map(|key| key.1));
                for event in events {
                    writer.send(SendType::Object(event));
                }
            }
            Incoming::ClientGone => return Ok(()),
        }
    }
    Ok(())
}

/// Reads client frames from stdin until it closes or sends garbage.
fn spawn_stdin(tx: mpsc::Sender<Incoming>) {
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        loop {
            let incoming = match protocol::read_frame::<ClientMessage>(&mut input) {
                Ok(framed) => Incoming::Client(framed.message),
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

fn send(out: &mut impl Write, message: HostMessage) -> Result<(), FrameError> {
    let framed = Framed::new(message);
    protocol::write_frame(out, &framed.message, &framed.payload)
}
