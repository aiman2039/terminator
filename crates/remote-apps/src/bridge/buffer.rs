//! wprs buffers and positioners as the compositor's types.

use wprs::serialization::wayland::{Buffer, BufferData, BufferFormat};
use wprs::serialization::xdg_shell::XdgPositioner;
use wprs::vec4u8::Vec4u8s;

use crate::compose::{Layer, Positioner};
use crate::protocol::Rect;

/// A surface's BGRA pixels.
pub(super) struct Pixels {
    pub(super) width: u32,
    pub(super) height: u32,
    stride: usize,
    opaque: bool,
    data: Vec<u8>,
}

impl Pixels {
    /// This buffer drawn at `x`,`y`.
    pub(super) fn layer(&self, x: i32, y: i32) -> Layer<'_> {
        Layer {
            x,
            y,
            width: self.width,
            height: self.height,
            stride: self.stride,
            pixels: &self.data,
            opaque: self.opaque,
        }
    }

    /// The whole buffer, at 0,0.
    pub(super) fn bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }
}

/// Decodes a wprs buffer. wprsd sends the bytes in a raw-buffer message just
/// before the commit, filtered for compression; `unfilter` restores BGRA.
pub(super) fn pixels(buffer: Buffer, pending: Option<Vec<u8>>) -> Option<Pixels> {
    let filtered = match buffer.data {
        BufferData::External => pending?,
        BufferData::Uncompressed(data) => data.0.into(),
        BufferData::Compressed(_) => return None,
    };
    let meta = buffer.metadata;
    let width = u32::try_from(meta.width).ok()?;
    let height = u32::try_from(meta.height).ok()?;
    let stride = usize::try_from(meta.stride).ok()?;
    let needed = stride.checked_mul(usize::try_from(height).ok()?)?;
    if filtered.len() < needed || !filtered.len().is_multiple_of(4) {
        return None;
    }
    let mut data = vec![0; filtered.len()];
    wprs::filtering::unfilter(&Vec4u8s::from(filtered), &mut data);
    Some(Pixels {
        width,
        height,
        stride,
        opaque: matches!(meta.format, BufferFormat::Xrgb8888),
        data,
    })
}

pub(super) fn positioner(p: &XdgPositioner) -> Positioner {
    Positioner {
        width: p.width,
        height: p.height,
        anchor_x: p.anchor_rect.loc.x,
        anchor_y: p.anchor_rect.loc.y,
        anchor_width: p.anchor_rect.size.w,
        anchor_height: p.anchor_rect.size.h,
        anchor: p.anchor_edges,
        gravity: p.gravity,
        offset_x: p.offset.x,
        offset_y: p.offset.y,
    }
}
