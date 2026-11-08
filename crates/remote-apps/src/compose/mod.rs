//! Platform-neutral pixel work for the host bridge: compositing a window's
//! surface tree into one BGRA image, and placing popups.

#[cfg(test)]
mod tests;

/// Bytes per BGRA pixel.
const BPP: usize = 4;

/// One surface buffer drawn at `x`,`y` in window coordinates. `pixels` holds
/// premultiplied BGRA rows (`ARGB8888` little-endian), `stride` bytes apart.
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pixels: &'a [u8],
    /// `XRGB8888`: the alpha byte is undefined and must be treated as 255.
    pub opaque: bool,
}

/// A tightly packed premultiplied BGRA image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    /// A fully transparent image. A size that does not fit in memory gives
    /// an empty canvas.
    pub fn new(width: u32, height: u32) -> Self {
        let len = row_bytes(width)
            .and_then(|row| row.checked_mul(usize::try_from(height).ok()?))
            .unwrap_or(0);
        let (width, height) = if len == 0 { (0, 0) } else { (width, height) };
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Draws `layer` over the canvas (source-over), clipped to the canvas.
    /// A layer whose buffer is shorter than its size says is skipped.
    pub fn draw(&mut self, layer: &Layer<'_>) {
        let Some(clip) = self.clip(layer) else {
            return;
        };
        let Some(dst_row) = row_bytes(self.width) else {
            return;
        };
        for row in 0..clip.rows {
            let Some(src) = clip
                .src_y
                .checked_add(row)
                .and_then(|y| offset(layer.stride, y, clip.src_x))
                .and_then(|start| layer.pixels.get(start..start.checked_add(clip.bytes)?))
            else {
                return;
            };
            let Some(dst) = clip
                .dst_y
                .checked_add(row)
                .and_then(|y| offset(dst_row, y, clip.dst_x))
                .and_then(|start| self.pixels.get_mut(start..start.checked_add(clip.bytes)?))
            else {
                return;
            };
            blend_row(dst, src, layer.opaque);
        }
    }

    fn clip(&self, layer: &Layer<'_>) -> Option<Clip> {
        let (src_x, dst_x, cols) = clip_axis(layer.x, layer.width, self.width)?;
        let (src_y, dst_y, rows) = clip_axis(layer.y, layer.height, self.height)?;
        Some(Clip {
            src_x,
            src_y,
            dst_x,
            dst_y,
            rows,
            bytes: cols.checked_mul(BPP)?,
        })
    }
}

struct Clip {
    src_x: usize,
    src_y: usize,
    dst_x: usize,
    dst_y: usize,
    rows: usize,
    bytes: usize,
}

/// Where a layer of `len` pixels at `pos` overlaps `0..limit`:
/// (first source pixel, first destination pixel, count).
fn clip_axis(pos: i32, len: u32, limit: u32) -> Option<(usize, usize, usize)> {
    let start = i64::from(pos).max(0);
    let end = i64::from(pos)
        .checked_add(i64::from(len))?
        .min(i64::from(limit));
    if end <= start {
        return None;
    }
    let skipped = start.checked_sub(i64::from(pos))?;
    Some((
        usize::try_from(skipped).ok()?,
        usize::try_from(start).ok()?,
        usize::try_from(end.checked_sub(start)?).ok()?,
    ))
}

fn row_bytes(width: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(BPP)
}

fn offset(stride: usize, row: usize, col: usize) -> Option<usize> {
    stride.checked_mul(row)?.checked_add(col.checked_mul(BPP)?)
}

fn blend_row(dst: &mut [u8], src: &[u8], opaque: bool) {
    for (d, s) in dst.chunks_exact_mut(BPP).zip(src.chunks_exact(BPP)) {
        let alpha = if opaque {
            255
        } else {
            s.get(3).copied().unwrap_or(255)
        };
        if alpha == 255 {
            d.copy_from_slice(s);
            if opaque && let Some(a) = d.get_mut(3) {
                *a = 255;
            }
            continue;
        }
        let keep = u16::from(255_u8.saturating_sub(alpha));
        for (dc, sc) in d.iter_mut().zip(s) {
            // keep <= 255 and *dc <= 255, so the product fits and the
            // quotient is <= 255.
            let below = u16::from(*dc).saturating_mul(keep) / 255;
            *dc = sc.saturating_add(u8::try_from(below).unwrap_or(u8::MAX));
        }
    }
}

/// xdg_positioner state, in the parent's window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Positioner {
    pub width: i32,
    pub height: i32,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: i32,
    pub anchor_height: i32,
    /// `xdg_positioner.anchor` value.
    pub anchor: u32,
    /// `xdg_positioner.gravity` value.
    pub gravity: u32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// Top-left of a popup in its parent's coordinates, without constraint
/// adjustment (the local window shows the whole parent, so popups are clipped
/// at worst, never lost).
pub fn popup_position(p: &Positioner) -> (i32, i32) {
    let (left, right, top, bottom) = edges(p.anchor);
    let ax = along(p.anchor_x, p.anchor_width, left, right);
    let ay = along(p.anchor_y, p.anchor_height, top, bottom);
    let (gleft, gright, gtop, gbottom) = edges(p.gravity);
    let x = ax.saturating_sub(extent_before(p.width, gleft, gright));
    let y = ay.saturating_sub(extent_before(p.height, gtop, gbottom));
    (x.saturating_add(p.offset_x), y.saturating_add(p.offset_y))
}

/// (left, right, top, bottom) for an xdg anchor or gravity value.
fn edges(value: u32) -> (bool, bool, bool, bool) {
    match value {
        1 => (false, false, true, false),
        2 => (false, false, false, true),
        3 => (true, false, false, false),
        4 => (false, true, false, false),
        5 => (true, false, true, false),
        6 => (true, false, false, true),
        7 => (false, true, true, false),
        8 => (false, true, false, true),
        _ => (false, false, false, false),
    }
}

/// The anchor point on one axis: the low edge, the high edge or the middle.
fn along(start: i32, len: i32, low: bool, high: bool) -> i32 {
    if low {
        start
    } else if high {
        start.saturating_add(len)
    } else {
        start.saturating_add(len / 2)
    }
}

/// How far the popup reaches before the anchor point on one axis.
fn extent_before(len: i32, toward_low: bool, toward_high: bool) -> i32 {
    if toward_low {
        len
    } else if toward_high {
        0
    } else {
        len / 2
    }
}
