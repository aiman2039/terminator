//! Terminator display protocol: the only format that crosses the network
//! between a host bridge and the local daemon.
//!
//! Each message is one frame:
//! `[u32 BE header length][JSON header][u32 BE payload length][payload]`.
//! The header is a [`HostMessage`] or a [`ClientMessage`]. The payload holds
//! bulk bytes (zstd pixels, cursor images, clipboard data) so they are not
//! JSON-escaped; it is empty for most messages.
//!
//! Both sides send `Hello` first. A side that sees a different
//! [`PROTOCOL_VERSION`] sends [`HostMessage::Error`] with
//! [`ErrorKind::VersionMismatch`] (or the client shows the same text) and
//! closes. Unknown capability strings are ignored, so new features are added
//! as capabilities, not as version bumps.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[cfg(test)]
mod tests;

/// Bump only for changes that old peers cannot ignore.
pub const PROTOCOL_VERSION: u32 = 1;
/// Largest JSON header. Titles and MIME lists are small.
pub const MAX_HEADER: usize = 1024 * 1024;
/// Largest payload: one compressed 8K BGRA frame fits with room.
pub const MAX_PAYLOAD: usize = 256 * 1024 * 1024;

/// Stable remote window identity for the life of the host bridge session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowRole {
    /// A normal app window: one Terminator tab.
    Toplevel,
    /// Menus, tooltips and dialogs: an overlay at `x`,`y` in the parent
    /// window's surface coordinates. Never a tab.
    Popup { parent: WindowId, x: i32, y: i32 },
}

/// Pixel encoding of a [`HostMessage::Frame`] payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameEncoding {
    /// zstd of the damaged rectangles, in order, each as tightly packed
    /// premultiplied BGRA rows (`width * 4` bytes per row).
    ZstdBgra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The two sides speak different [`PROTOCOL_VERSION`]s. The text tells
    /// the user which Terminator install to update.
    VersionMismatch,
    /// `wprsd` is not installed in the host's Terminator install.
    WprsdMissing,
    /// `wprsd` stopped. Apps that were connected to it are gone.
    WprsdStopped,
    /// Xwayland is not installed, so X11 apps cannot start.
    XwaylandMissing,
    /// The host CPU or OS is not supported.
    Unsupported,
    /// Any other failure; the message has the detail.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Hello {
        protocol: u32,
        /// The host's Terminator version, shown in update hints.
        version: String,
        /// `DISPLAY` for X11 apps, for example `:5`. `None` without Xwayland.
        display: Option<String>,
        /// `WAYLAND_DISPLAY` for native Wayland apps.
        wayland_display: String,
        capabilities: Vec<String>,
    },
    WindowOpened {
        id: WindowId,
        role: WindowRole,
        title: String,
        app_id: String,
    },
    WindowChanged {
        id: WindowId,
        title: String,
        app_id: String,
    },
    /// New pixels. The payload is encoded as `encoding` says.
    Frame {
        id: WindowId,
        width: u32,
        height: u32,
        damage: Vec<Rect>,
        encoding: FrameEncoding,
    },
    WindowClosed {
        id: WindowId,
    },
    /// The payload is the cursor image as premultiplied BGRA. A zero-size
    /// image with no `name` hides the cursor. With `name` (a CSS cursor
    /// name such as `text`), the client uses its own cursor of that shape.
    Cursor {
        width: u32,
        height: u32,
        hotspot_x: i32,
        hotspot_y: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// A remote app took the clipboard and offers these types.
    ClipboardOffer {
        mime_types: Vec<String>,
    },
    /// Answer to [`ClientMessage::ClipboardRead`]; the payload is the data.
    ClipboardData {
        mime_type: String,
    },
    Error {
        kind: ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        protocol: u32,
        version: String,
        capabilities: Vec<String>,
    },
    /// The size the tab offers the window, in logical pixels, and its state.
    Configure {
        id: WindowId,
        width: u32,
        height: u32,
        scale: u32,
        focused: bool,
    },
    /// The user closed the tab and asked the app to close (not a detach).
    CloseRequested {
        id: WindowId,
    },
    PointerMotion {
        id: WindowId,
        x: f64,
        y: f64,
    },
    PointerLeave {
        id: WindowId,
    },
    PointerButton {
        id: WindowId,
        button: PointerButton,
        pressed: bool,
    },
    Scroll {
        id: WindowId,
        dx: f64,
        dy: f64,
        /// Wheel clicks; zero for touchpad scrolling.
        discrete_x: i32,
        discrete_y: i32,
    },
    /// `keycode` is a Linux evdev code; the host maps it with its keymap.
    Key {
        id: WindowId,
        keycode: u32,
        pressed: bool,
    },
    Modifiers {
        shift: bool,
        ctrl: bool,
        alt: bool,
        logo: bool,
        caps_lock: bool,
        num_lock: bool,
    },
    /// The local clipboard changed; these types can be read with a
    /// [`HostMessage`] round trip later. Sent only when the host allows sync.
    ClipboardOffer {
        mime_types: Vec<String>,
    },
    ClipboardRead {
        mime_type: String,
    },
    /// Answer to a host read of the local clipboard; the payload is the data.
    ClipboardData {
        mime_type: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("connection closed")]
    Closed,
    #[error("frame header is {0} bytes, the limit is {MAX_HEADER}")]
    HeaderTooLarge(usize),
    #[error("frame payload is {0} bytes, the limit is {MAX_PAYLOAD}")]
    PayloadTooLarge(usize),
    #[error("bad frame header: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// One decoded message and its bulk payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Framed<M> {
    pub message: M,
    pub payload: Vec<u8>,
}

impl<M> Framed<M> {
    pub fn new(message: M) -> Self {
        Self {
            message,
            payload: Vec::new(),
        }
    }

    pub fn with_payload(message: M, payload: Vec<u8>) -> Self {
        Self { message, payload }
    }
}

pub fn write_frame<M: Serialize>(
    out: &mut impl Write,
    message: &M,
    payload: &[u8],
) -> Result<(), FrameError> {
    let header = serde_json::to_vec(message)?;
    if header.len() > MAX_HEADER {
        return Err(FrameError::HeaderTooLarge(header.len()));
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::PayloadTooLarge(payload.len()));
    }
    out.write_all(&length_prefix(header.len())?)?;
    out.write_all(&header)?;
    out.write_all(&length_prefix(payload.len())?)?;
    out.write_all(payload)?;
    out.flush()?;
    Ok(())
}

/// Reads one frame. A clean end of stream before a frame starts is
/// [`FrameError::Closed`]; an end inside a frame is an I/O error.
pub fn read_frame<M: for<'de> Deserialize<'de>>(
    input: &mut impl Read,
) -> Result<Framed<M>, FrameError> {
    let header_len = match read_length(input) {
        Err(FrameError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::Closed);
        }
        other => other?,
    };
    if header_len > MAX_HEADER {
        return Err(FrameError::HeaderTooLarge(header_len));
    }
    let header = read_exact_vec(input, header_len)?;
    let payload_len = read_length(input)?;
    if payload_len > MAX_PAYLOAD {
        return Err(FrameError::PayloadTooLarge(payload_len));
    }
    let payload = read_exact_vec(input, payload_len)?;
    Ok(Framed {
        message: serde_json::from_slice(&header)?,
        payload,
    })
}

/// The text both sides show when the versions differ. It names the side to
/// update, because the user cannot see the other side's version.
pub fn version_mismatch_message(
    host_protocol: u32,
    host_version: &str,
    client_protocol: u32,
) -> String {
    if host_protocol < client_protocol {
        format!(
            "The host's Terminator ({host_version}) is too old for remote apps. Update Terminator on the host."
        )
    } else {
        format!(
            "The host's Terminator ({host_version}) is newer than this Terminator. Update Terminator on this computer."
        )
    }
}

fn length_prefix(len: usize) -> Result<[u8; 4], FrameError> {
    u32::try_from(len)
        .map(u32::to_be_bytes)
        .map_err(|_| FrameError::PayloadTooLarge(len))
}

fn read_length(input: &mut impl Read) -> Result<usize, FrameError> {
    let mut bytes = [0_u8; 4];
    input.read_exact(&mut bytes)?;
    usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| FrameError::PayloadTooLarge(usize::MAX))
}

fn read_exact_vec(input: &mut impl Read, len: usize) -> Result<Vec<u8>, FrameError> {
    let mut bytes = vec![0_u8; len];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}
