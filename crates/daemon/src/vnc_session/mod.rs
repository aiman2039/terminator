mod connection;
pub(crate) mod credentials;
mod dirty;
mod input;
mod quality;
mod reconnect;
pub(crate) mod shm;
mod ssh_steps;
mod stats;
mod thumbnail;
mod transfer;
mod transport;

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use flate2::{Compression, write::ZlibEncoder};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{
    id, now,
    vnc::{VncFrame, VncInputEvent, VncRegion, VncSession, VncSessionStatus},
};
use tokio::sync::mpsc;
use vnc::{ClientKeyEvent, PixelFormat, Rect, VncConnector, VncEvent, X11Event};

use crate::server::{Shared, relock};

const MAX_DIMENSION: u16 = 8192;
/// A viewer that has not asked for a frame within this window has stopped
/// looking (its window is hidden or closed): the server refresh stops too,
/// so an unwatched session costs no traffic.
const CLIENT_ACTIVE: Duration = Duration::from_secs(2);

pub(crate) struct VncRuntime {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) rgba: Vec<u8>,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    pub(crate) clipboard: Option<String>,
    pub(crate) view_only: bool,
    pub(crate) input_tx: Option<mpsc::Sender<VncInputEvent>>,
    pub(crate) options: terminator_core::vnc::VncOptions,
    pub(crate) cursor: Option<terminator_core::vnc::VncCursor>,
    pub(crate) monitors: Vec<terminator_core::vnc::VncMonitor>,
    pub(crate) resize_supported: bool,
    pub(crate) last_encoded_bytes: usize,
    pub(crate) transfers: HashMap<String, terminator_core::vnc::VncTransferStatus>,
    /// Union of framebuffer rectangles changed since the last encode; see
    /// [`VncRuntime::encode_frame`]. `None` means the whole frame is stale.
    dirty: Option<VncRegion>,
    /// Revision the last encoded frame was cut from: a client that reports
    /// this revision can take an incremental rectangle.
    encoded_revision: u64,
    last_poll: Option<Instant>,
    /// Shared-memory framebuffer for `vnc-shm-v1` viewers; created on the
    /// first such request and replaced on resize. Dropping it removes the file.
    shm: Option<terminator_sys::SharedFrameWriter>,
    /// Rectangles changed since the last shared-memory publish.
    shm_dirty: dirty::DirtyRects,
    /// Revision the shared-memory file holds; 0 before the first publish.
    shm_revision: u64,
    /// Files created so far; each resize gets a new file name.
    shm_files: u32,
    /// Bumped whenever the cursor image changes.
    pub(crate) cursor_serial: u64,
    pub(crate) stats: stats::UpdateStats,
    /// An incremental update request is out and its update has not ended.
    update_pending: bool,
}

impl VncRuntime {
    fn new(view_only: bool, input_tx: mpsc::Sender<VncInputEvent>) -> Self {
        Self {
            width: 0,
            height: 0,
            rgba: Vec::new(),
            revision: 1,
            status: VncSessionStatus::Connecting,
            error: None,
            clipboard: None,
            view_only,
            input_tx: Some(input_tx),
            options: terminator_core::vnc::VncOptions::default(),
            cursor: None,
            monitors: Vec::new(),
            resize_supported: false,
            last_encoded_bytes: 0,
            transfers: HashMap::new(),
            dirty: None,
            encoded_revision: 0,
            last_poll: None,
            shm: None,
            shm_dirty: dirty::DirtyRects::Full,
            shm_revision: 0,
            shm_files: 0,
            cursor_serial: 0,
            stats: stats::UpdateStats::default(),
            update_pending: false,
        }
    }

    fn resize(&mut self, width: u16, height: u16) {
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || usize::from(width).saturating_mul(usize::from(height)) > 32 * 1024 * 1024
        {
            self.status = VncSessionStatus::Failed;
            self.error = Some("Remote desktop exceeds the supported dimensions".into());
            return;
        }
        if self.width == width && self.height == height && !self.rgba.is_empty() {
            return;
        }
        self.width = width;
        self.height = height;
        let len = usize::from(width)
            .saturating_mul(usize::from(height))
            .saturating_mul(4);
        self.rgba.clear();
        self.rgba.resize(len, 0);
        for chunk in self.rgba.chunks_exact_mut(4) {
            if let Some(alpha) = chunk.get_mut(3) {
                *alpha = 255;
            }
        }
        // Every pixel changed: only a full frame can describe this now.
        self.dirty = None;
        self.shm_dirty = dirty::DirtyRects::Full;
        self.revision = self.revision.saturating_add(1);
    }

    fn mark_dirty(&mut self, region: VncRegion) {
        if region.is_empty() {
            return;
        }
        self.shm_dirty.add(region);
        self.dirty = Some(match self.dirty {
            Some(previous) => previous.union(region),
            None => region,
        });
    }

    fn blit_rgba(&mut self, rect: Rect, src: &[u8]) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let dst_w = usize::from(self.width);
        let dst_h = usize::from(self.height);
        let rx = usize::from(rect.x);
        let ry = usize::from(rect.y);
        let rw = usize::from(rect.width);
        let rh = usize::from(rect.height);
        if rx >= dst_w || ry >= dst_h || rw == 0 || rh == 0 {
            return;
        }
        let copy_w = rw.min(dst_w.saturating_sub(rx));
        let copy_h = rh.min(dst_h.saturating_sub(ry));
        let src_row_bytes = rw.saturating_mul(4);
        let copy_row_bytes = copy_w.saturating_mul(4);
        for row in 0..copy_h {
            let src_start = row.saturating_mul(src_row_bytes);
            let src_end = src_start.saturating_add(copy_row_bytes);
            let dst_y = ry.saturating_add(row);
            let dst_start = dst_y
                .saturating_mul(dst_w)
                .saturating_add(rx)
                .saturating_mul(4);
            let dst_end = dst_start.saturating_add(copy_row_bytes);
            if let (Some(src_slice), Some(dst_slice)) = (
                src.get(src_start..src_end),
                self.rgba.get_mut(dst_start..dst_end),
            ) {
                dst_slice.copy_from_slice(src_slice);
                for pixel in dst_slice.chunks_exact_mut(4) {
                    if let Some(alpha) = pixel.get_mut(3) {
                        *alpha = 255;
                    }
                }
            }
        }
        self.mark_dirty(VncRegion {
            x: rect.x,
            y: rect.y,
            width: u16::try_from(copy_w).unwrap_or(0),
            height: u16::try_from(copy_h).unwrap_or(0),
        });
        self.revision = self.revision.saturating_add(1);
    }

    fn copy_rect(&mut self, dst: Rect, src: Rect) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let fb_w = usize::from(self.width);
        let fb_h = usize::from(self.height);
        let rw = usize::from(dst.width);
        let rh = usize::from(dst.height);
        let sx = usize::from(src.x);
        let sy = usize::from(src.y);
        if sx >= fb_w || sy >= fb_h || rw == 0 || rh == 0 {
            return;
        }
        let copy_w = rw.min(fb_w.saturating_sub(sx));
        let copy_h = rh.min(fb_h.saturating_sub(sy));
        let row_bytes = copy_w.saturating_mul(4);
        let mut temp = vec![0u8; copy_h.saturating_mul(row_bytes)];
        for row in 0..copy_h {
            let src_y = sy.saturating_add(row);
            let src_start = src_y
                .saturating_mul(fb_w)
                .saturating_add(sx)
                .saturating_mul(4);
            let src_end = src_start.saturating_add(row_bytes);
            let tmp_start = row.saturating_mul(row_bytes);
            let tmp_end = tmp_start.saturating_add(row_bytes);
            if let (Some(s), Some(t)) = (
                self.rgba.get(src_start..src_end),
                temp.get_mut(tmp_start..tmp_end),
            ) {
                t.copy_from_slice(s);
            }
        }
        let clipped = Rect {
            x: dst.x,
            y: dst.y,
            width: u16::try_from(copy_w).unwrap_or(0),
            height: u16::try_from(copy_h).unwrap_or(0),
        };
        self.blit_rgba(clipped, &temp);
    }

    /// Encode the current frame for `session`. With `regions` and a client
    /// that is exactly at the last encoded revision, only the rectangle
    /// changed since that encode is compressed; otherwise the whole
    /// framebuffer. Either way the dirty union is consumed and the encoded
    /// revision becomes current, so the next request can diff against it.
    pub(crate) fn encode_frame(
        &mut self,
        session: &str,
        since: u64,
        regions: bool,
    ) -> Result<VncFrame> {
        let region = if regions && since > 0 && since == self.encoded_revision {
            self.dirty.filter(|region| !region.is_empty())
        } else {
            None
        };
        let mut frame = VncFrame {
            cursor: self.cursor.clone(),
            monitors: self.monitors.clone(),
            resize_supported: self.resize_supported,
            shm: None,
            cursor_serial: self.cursor_serial,
            session: session.to_owned(),
            width: self.width,
            height: self.height,
            revision: self.revision,
            status: self.status,
            error: self.error.clone(),
            zlib_rgba: String::new(),
            region: None,
            clipboard: if self.options.clipboard.receive() {
                self.clipboard.clone()
            } else {
                None
            },
        };
        if let Some((region, pixels)) = region.and_then(|region| self.region_pixels(region)) {
            frame.zlib_rgba = encode_pixels(&pixels)?;
            frame.region = Some(region);
        } else if self.width > 0 && self.height > 0 && !self.rgba.is_empty() {
            frame.zlib_rgba = encode_pixels(&self.rgba)?;
        }
        self.last_encoded_bytes = frame.zlib_rgba.len();
        self.dirty = None;
        self.encoded_revision = self.revision;
        Ok(frame)
    }

    /// Copy one rectangle out of the framebuffer, clipped to its bounds.
    /// `None` when the rectangle is outside or the framebuffer is empty.
    fn region_pixels(&self, region: VncRegion) -> Option<(VncRegion, Vec<u8>)> {
        if self.width == 0 || self.height == 0 || region.is_empty() {
            return None;
        }
        let fb_w = usize::from(self.width);
        let fb_h = usize::from(self.height);
        let x = usize::from(region.x).min(fb_w);
        let y = usize::from(region.y).min(fb_h);
        let width = usize::from(region.width).min(fb_w.saturating_sub(x));
        let height = usize::from(region.height).min(fb_h.saturating_sub(y));
        if width == 0 || height == 0 {
            return None;
        }
        let row_bytes = width.saturating_mul(4);
        let mut pixels = Vec::with_capacity(height.saturating_mul(row_bytes));
        for row in 0..height {
            let start = y
                .saturating_add(row)
                .saturating_mul(fb_w)
                .saturating_add(x)
                .saturating_mul(4);
            let end = start.saturating_add(row_bytes);
            pixels.extend_from_slice(self.rgba.get(start..end)?);
        }
        Some((
            VncRegion {
                x: u16::try_from(x).unwrap_or(0),
                y: u16::try_from(y).unwrap_or(0),
                width: u16::try_from(width).unwrap_or(0),
                height: u16::try_from(height).unwrap_or(0),
            },
            pixels,
        ))
    }

    /// True while a viewer has asked for a frame within [`CLIENT_ACTIVE`].
    fn polled_recently(&self) -> bool {
        self.last_poll
            .is_some_and(|polled| polled.elapsed() < CLIENT_ACTIVE)
    }
}

fn encode_pixels(pixels: &[u8]) -> Result<String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(pixels)?;
    Ok(B64.encode(encoder.finish()?))
}

pub(crate) struct VncConnectSpec {
    pub(crate) options: terminator_core::vnc::VncOptions,
    pub(crate) project: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) shared: bool,
    pub(crate) view_only: bool,
    pub(crate) label: Option<String>,
}

fn validate_project(state: &terminator_core::State, project: &str) -> Result<()> {
    ensure!(
        project.is_empty()
            || state.projects.iter().any(|p| p.id == project)
            || state.remote_projects.iter().any(|p| p.id == project),
        "Unknown project"
    );
    Ok(())
}

mod connect;
pub(crate) use connect::VncFrameRequest;
struct RunVncThreadInput {
    shared: Arc<Shared>,
    runtime: Arc<Mutex<VncRuntime>>,
    session_id: String,
    spec: connection::ConnectionSpec,
    input_rx: mpsc::Receiver<VncInputEvent>,
}
fn run_vnc_thread(input: RunVncThreadInput) {
    let RunVncThreadInput {
        shared,
        runtime,
        session_id,
        spec,
        input_rx,
    } = input;
    let shared = &shared;
    let runtime = &runtime;
    let session_id = &session_id;
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        mark_failed(MarkFailedInput {
            shared,
            runtime,
            session_id,
            message: "Could not initialize async runtime".into(),
            kind: terminator_core::vnc::VncErrorKind::Other,
        });
        return;
    };
    rt.block_on(reconnect::run(reconnect::RunInput {
        shared: Arc::clone(shared),
        runtime: Arc::clone(runtime),
        sid: session_id.to_owned(),
        spec,
        input_rx,
    }));
}

mod events;
use events::{ApplyVncEventInput, apply_vnc_event};
mod status;
use status::{
    MarkDisconnectedInput, MarkFailedInput, mark_connected, mark_disconnected, mark_failed,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod shm_tests;
