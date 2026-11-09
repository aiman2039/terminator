//! Pixel helpers for bridge messages: frame decoding and cursor images.

use base64::Engine as _;
use terminator_core::remote_apps::protocol;
use terminator_core::remote_apps::{HostMessage, RemoteCursor};

/// A `Frame`'s damage rectangles as RGBA, decoded off the session lock
/// (decompression takes milliseconds for large windows).
pub(crate) fn decode_frame(framed: &protocol::Framed<HostMessage>) -> Option<Vec<Vec<u8>>> {
    let HostMessage::Frame { damage, .. } = &framed.message else {
        return None;
    };
    terminator_core::remote_apps::zstd_bgra_to_rgba(&framed.payload, damage)
}

/// A cursor message as RGBA (base64); `None` for a bad size.
pub(super) fn cursor(
    mut rgba: Vec<u8>,
    (width, height): (u32, u32),
    (hotspot_x, hotspot_y): (i32, i32),
    name: Option<String>,
) -> Option<RemoteCursor> {
    let (width, height) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
    let expected = usize::from(width)
        .saturating_mul(usize::from(height))
        .saturating_mul(4);
    if rgba.len() != expected {
        rgba.clear();
    }
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(RemoteCursor {
        serial: 0,
        width,
        height,
        hotspot_x,
        hotspot_y,
        name,
        rgba: base64::engine::general_purpose::STANDARD.encode(rgba),
    })
}
