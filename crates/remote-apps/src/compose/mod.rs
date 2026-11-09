//! Platform-neutral pixel work: compositing a window's surface tree into one
//! BGRA image and placing popups (host bridge), and decoding frames (daemon).

#[cfg(test)]
mod tests;

use crate::protocol::Rect;

/// Bytes per BGRA pixel.
const BPP: usize = 4;

/// Decodes a [`crate::protocol::FrameEncoding::ZstdBgra`] payload holding
/// `rects` (in order, packed rows) into premultiplied RGBA, one buffer per
/// rectangle. `None` when the payload is corrupt or not exactly that size.
pub fn zstd_bgra_to_rgba(payload: &[u8], rects: &[Rect]) -> Option<Vec<Vec<u8>>> {
    let lens: Vec<usize> = rects.iter().map(rect_bytes).collect::<Option<_>>()?;
    let total = lens
        .iter()
        .try_fold(0_usize, |sum, len| sum.checked_add(*len))?;
    let mut pixels = zstd::bulk::decompress(payload, total).ok()?;
    if pixels.len() != total {
        return None;
    }
    for pixel in pixels.chunks_exact_mut(BPP) {
        pixel.swap(0, 2);
    }
    if let [_] = lens.as_slice() {
        // The usual case: one rectangle, no copy.
        return Some(vec![pixels]);
    }
    let mut out = Vec::with_capacity(lens.len());
    let mut rest = pixels.as_slice();
    for len in lens {
        let (head, tail) = rest.split_at_checked(len)?;
        out.push(head.to_vec());
        rest = tail;
    }
    Some(out)
}

fn rect_bytes(rect: &Rect) -> Option<usize> {
    row_bytes(rect.width)?.checked_mul(usize::try_from(rect.height).ok()?)
}

/// The smallest rectangle holding both.
pub fn union(a: Rect, b: Rect) -> Rect {
    if a.width == 0 || a.height == 0 {
        return b;
    }
    if b.width == 0 || b.height == 0 {
        return a;
    }
    let right = |r: Rect| i64::from(r.x).saturating_add(i64::from(r.width));
    let bottom = |r: Rect| i64::from(r.y).saturating_add(i64::from(r.height));
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let width = right(a).max(right(b)).saturating_sub(i64::from(x));
    let height = bottom(a).max(bottom(b)).saturating_sub(i64::from(y));
    Rect {
        x,
        y,
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
    }
}

/// `rect` cut to a `width` x `height` image; `None` when nothing is left.
pub fn clamp(rect: Rect, width: u32, height: u32) -> Option<Rect> {
    let (_, x, w) = clip_axis(rect.x, rect.width, (0, width))?;
    let (_, y, h) = clip_axis(rect.y, rect.height, (0, height))?;
    Some(Rect {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        width: u32::try_from(w).ok()?,
        height: u32::try_from(h).ok()?,
    })
}

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

    /// The whole canvas as a rectangle.
    pub fn bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    /// Makes `rect` fully transparent.
    pub fn clear(&mut self, rect: Rect) {
        let Some(rect) = clamp(rect, self.width, self.height) else {
            return;
        };
        for row in self.rows_mut(rect) {
            row.fill(0);
        }
    }

    /// The pixels of `rect` (inside the canvas) as packed rows.
    pub fn region(&self, rect: Rect) -> Vec<u8> {
        clamp(rect, self.width, self.height)
            .map(|rect| self.rows(rect).flatten().copied().collect())
            .unwrap_or_default()
    }

    /// Row geometry of `rect` (inside the canvas): row bytes, skipped rows,
    /// rows, and the byte range in each row.
    fn row_span(&self, rect: Rect) -> (usize, usize, usize, std::ops::Range<usize>) {
        let row = row_bytes(self.width).unwrap_or(0).max(1);
        let x = usize::try_from(rect.x).unwrap_or(0).saturating_mul(BPP);
        let bytes = row_bytes(rect.width).unwrap_or(0);
        let y = usize::try_from(rect.y).unwrap_or(0);
        let height = usize::try_from(rect.height).unwrap_or(0);
        (row, y, height, x..x.saturating_add(bytes))
    }

    fn rows(&self, rect: Rect) -> impl Iterator<Item = &[u8]> {
        let (row, y, height, span) = self.row_span(rect);
        self.pixels
            .chunks_exact(row)
            .skip(y)
            .take(height)
            .filter_map(move |line| line.get(span.clone()))
    }

    fn rows_mut(&mut self, rect: Rect) -> impl Iterator<Item = &mut [u8]> {
        let (row, y, height, span) = self.row_span(rect);
        self.pixels
            .chunks_exact_mut(row)
            .skip(y)
            .take(height)
            .filter_map(move |line| line.get_mut(span.clone()))
    }

    /// Draws `layer` over the canvas (source-over), clipped to the canvas.
    /// A layer whose buffer is shorter than its size says is skipped.
    pub fn draw(&mut self, layer: &Layer<'_>) {
        self.draw_in(layer, self.bounds());
    }

    /// [`Canvas::draw`] limited to `area`.
    pub fn draw_in(&mut self, layer: &Layer<'_>, area: Rect) {
        let Some(area) = clamp(area, self.width, self.height) else {
            return;
        };
        let Some(clip) = clip(layer, area) else {
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
}

/// Where `layer` overlaps `area` (already inside the canvas).
fn clip(layer: &Layer<'_>, area: Rect) -> Option<Clip> {
    let limits = |pos: i32, len: u32| {
        let start = u32::try_from(pos).ok()?;
        Some((start, start.checked_add(len)?))
    };
    let x_limits = limits(area.x, area.width)?;
    let y_limits = limits(area.y, area.height)?;
    let (src_x, dst_x, cols) = clip_axis(layer.x, layer.width, x_limits)?;
    let (src_y, dst_y, rows) = clip_axis(layer.y, layer.height, y_limits)?;
    Some(Clip {
        src_x,
        src_y,
        dst_x,
        dst_y,
        rows,
        bytes: cols.checked_mul(BPP)?,
    })
}

struct Clip {
    src_x: usize,
    src_y: usize,
    dst_x: usize,
    dst_y: usize,
    rows: usize,
    bytes: usize,
}

/// Where a layer of `len` pixels at `pos` overlaps `low..high`:
/// (first source pixel, first destination pixel, count).
fn clip_axis(pos: i32, len: u32, (low, high): (u32, u32)) -> Option<(usize, usize, usize)> {
    let start = i64::from(pos).max(i64::from(low));
    let end = i64::from(pos)
        .checked_add(i64::from(len))?
        .min(i64::from(high));
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
