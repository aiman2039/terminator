use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use flate2::{Compression, write::ZlibEncoder};
use std::{
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
use vnc::{
    ClientKeyEvent, ClientMouseEvent, PixelFormat, Rect, VncConnector, VncEncoding, VncEvent,
    X11Event,
};

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
    pub(crate) input_tx: Option<mpsc::UnboundedSender<VncInputEvent>>,
    /// Union of framebuffer rectangles changed since the last encode; see
    /// [`VncRuntime::encode_frame`]. `None` means the whole frame is stale.
    dirty: Option<VncRegion>,
    /// Revision the last encoded frame was cut from: a client that reports
    /// this revision can take an incremental rectangle.
    encoded_revision: u64,
    last_poll: Option<Instant>,
}

impl VncRuntime {
    fn new(view_only: bool, input_tx: mpsc::UnboundedSender<VncInputEvent>) -> Self {
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
            dirty: None,
            encoded_revision: 0,
            last_poll: None,
        }
    }

    fn resize(&mut self, width: u16, height: u16) {
        let width = width.min(MAX_DIMENSION);
        let height = height.min(MAX_DIMENSION);
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
        self.revision = self.revision.saturating_add(1);
    }

    fn mark_dirty(&mut self, region: VncRegion) {
        if region.is_empty() {
            return;
        }
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
            session: session.to_owned(),
            width: self.width,
            height: self.height,
            revision: self.revision,
            status: self.status,
            error: self.error.clone(),
            zlib_rgba: String::new(),
            region: None,
            clipboard: self.clipboard.clone(),
        };
        if let Some((region, pixels)) = region.and_then(|region| self.region_pixels(region)) {
            frame.zlib_rgba = encode_pixels(&pixels)?;
            frame.region = Some(region);
        } else if self.width > 0 && self.height > 0 && !self.rgba.is_empty() {
            frame.zlib_rgba = encode_pixels(&self.rgba)?;
        }
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

impl Shared {
    pub(crate) fn vnc_connect(self: &Arc<Self>, spec: VncConnectSpec) -> Result<VncSession> {
        let VncConnectSpec {
            project,
            profile_id,
            host,
            port,
            mut username,
            mut password,
            mut shared,
            mut view_only,
            label,
        } = spec;
        let host = host.trim().to_owned();
        ensure!(!host.is_empty() && host.len() <= 255, "Invalid VNC host");
        ensure!(port > 0, "Invalid VNC port");
        let sid = id();
        let now_ts = now();
        // A reconnect to the same endpoint leaves the previous dead record
        // behind forever; drop it (and its runtime) so the sidebar and the
        // session list do not fill with duplicates.
        let mut stale: Vec<String> = Vec::new();
        let record = {
            let mut state = relock(&self.state);
            if !project.is_empty() {
                ensure!(
                    state.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
            }
            state.vnc_sessions.retain(|session| {
                let duplicate = session.project_id == project
                    && session.host == host
                    && session.port == port
                    && !session.status.live();
                if duplicate {
                    stale.push(session.id.clone());
                }
                !duplicate
            });
            if let Some(pid) = &profile_id
                && let Some(profile) = state.vnc_profiles.iter_mut().find(|p| &p.id == pid)
            {
                if password.is_empty() {
                    password.clone_from(&profile.password);
                }
                if username.is_empty() {
                    username.clone_from(&profile.username);
                }
                shared = profile.shared;
                view_only = profile.view_only;
                profile.last_connected = now_ts;
            }
            let resolved_label = label
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| format!("{host}:{port}"));
            let record = VncSession {
                id: sid.clone(),
                project_id: project,
                profile_id,
                label: resolved_label,
                host: host.clone(),
                port,
                width: 0,
                height: 0,
                view_only,
                status: VncSessionStatus::Connecting,
                error: None,
                created: now_ts,
                generation: state.generation.clone(),
            };
            state.vnc_sessions.push(record.clone());
            state.revision = state.revision.saturating_add(1);
            record
        };
        if let Some(paths) = &self.catalog_paths {
            let _ = terminator_core::generations::Catalog::open(paths)
                .and_then(|c| c.save_workspace(&relock(&self.state)));
        }
        let _ = self.persist();

        for id in &stale {
            if let Some(runtime) = relock(&self.vnc_sessions).remove(id) {
                let mut rt = relock(&runtime);
                rt.input_tx = None;
                rt.status = VncSessionStatus::Disconnected;
                rt.revision = rt.revision.saturating_add(1);
            }
        }

        // Unbounded: input must never be dropped or block the RPC thread,
        // and an unbounded send from an async context is immediate.
        let (input_tx, input_rx) = mpsc::unbounded_channel::<VncInputEvent>();
        let runtime = Arc::new(Mutex::new(VncRuntime::new(view_only, input_tx)));
        relock(&self.vnc_sessions).insert(sid.clone(), Arc::clone(&runtime));

        let shared_ref = Arc::clone(self);
        let session_id = sid;
        thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_vnc_thread(
                    &shared_ref,
                    &runtime,
                    &session_id,
                    host,
                    port,
                    password,
                    shared,
                    view_only,
                    input_rx,
                );
            }));
            if outcome.is_err() {
                // The session thread is gone: the GUI must not keep showing
                // a live connection that will never produce another frame.
                mark_failed(
                    &shared_ref,
                    &runtime,
                    &session_id,
                    "VNC session stopped after an internal error".into(),
                );
            }
        });

        Ok(record)
    }

    pub(crate) fn vnc_disconnect(&self, session: &str) -> Result<()> {
        if let Some(runtime) = relock(&self.vnc_sessions).remove(session) {
            let mut rt = relock(&runtime);
            rt.input_tx = None;
            rt.status = VncSessionStatus::Disconnected;
            rt.revision = rt.revision.saturating_add(1);
        }
        let mut state = relock(&self.state);
        if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session) {
            rec.status = VncSessionStatus::Disconnected;
        }
        state.vnc_sessions.retain(|s| s.id != session);
        state.revision = state.revision.saturating_add(1);
        Ok(())
    }

    pub(crate) fn vnc_input(&self, session: &str, events: Vec<VncInputEvent>) -> Result<()> {
        let runtime = relock(&self.vnc_sessions)
            .get(session)
            .cloned()
            .context("VNC session is not live")?;
        let tx = {
            let rt = relock(&runtime);
            if rt.view_only {
                return Ok(());
            }
            rt.input_tx.clone().context("VNC session is closed")?
        };
        for event in events {
            let _ = tx.send(event);
        }
        Ok(())
    }

    pub(crate) fn vnc_frame(
        &self,
        session: &str,
        since: u64,
        regions: bool,
    ) -> Result<Option<VncFrame>> {
        let runtime = relock(&self.vnc_sessions)
            .get(session)
            .cloned()
            .context("VNC session is not live")?;
        let mut rt = relock(&runtime);
        // Any request counts as the viewer still watching: hidden or closed
        // views stop asking, and the refresh below stops with them.
        rt.last_poll = Some(Instant::now());
        if rt.revision <= since {
            return Ok(None);
        }
        rt.encode_frame(session, since, regions).map(Some)
    }
}

/// Build the per-session runtime and drive it to completion. Split out so
/// the spawning thread can catch a panic from anywhere inside, including
/// the runtime build.
#[allow(clippy::too_many_arguments)]
fn run_vnc_thread(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    host: String,
    port: u16,
    password: String,
    allow_shared: bool,
    view_only: bool,
    input_rx: mpsc::UnboundedReceiver<VncInputEvent>,
) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        mark_failed(
            shared,
            runtime,
            session_id,
            "Could not initialize async runtime".into(),
        );
        return;
    };
    rt.block_on(run_vnc_session(
        Arc::clone(shared),
        Arc::clone(runtime),
        session_id.to_owned(),
        host,
        port,
        password,
        allow_shared,
        view_only,
        input_rx,
    ));
}

#[allow(clippy::too_many_arguments)]
async fn run_vnc_session(
    shared: Arc<Shared>,
    runtime: Arc<Mutex<VncRuntime>>,
    session_id: String,
    host: String,
    port: u16,
    password: String,
    allow_shared: bool,
    view_only: bool,
    mut input_rx: mpsc::UnboundedReceiver<VncInputEvent>,
) {
    let endpoint = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let tcp = match tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(&endpoint),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => {
            mark_failed(
                &shared,
                &runtime,
                &session_id,
                format!("Connection failed: {err}"),
            );
            return;
        }
        Err(_) => {
            mark_failed(
                &shared,
                &runtime,
                &session_id,
                format!("Connection to {endpoint} timed out"),
            );
            return;
        }
    };
    let _ = tcp.set_nodelay(true);
    let connector = VncConnector::new(tcp)
        .set_auth_method(async move { Ok(password) })
        .add_encoding(VncEncoding::Tight)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::Raw)
        .add_encoding(VncEncoding::DesktopSizePseudo)
        .allow_shared(allow_shared)
        .set_pixel_format(PixelFormat::rgba());

    let client = match connector.build() {
        Ok(state) => match tokio::time::timeout(Duration::from_secs(15), state.try_start()).await {
            Ok(Ok(started)) => match started.finish() {
                Ok(client) => client,
                Err(err) => {
                    mark_failed(
                        &shared,
                        &runtime,
                        &session_id,
                        format!("VNC handshake failed: {err}"),
                    );
                    return;
                }
            },
            Ok(Err(err)) => {
                mark_failed(
                    &shared,
                    &runtime,
                    &session_id,
                    format!("VNC authentication failed: {err}"),
                );
                return;
            }
            Err(_) => {
                mark_failed(
                    &shared,
                    &runtime,
                    &session_id,
                    "VNC authentication timed out".into(),
                );
                return;
            }
        },
        Err(err) => {
            mark_failed(
                &shared,
                &runtime,
                &session_id,
                format!("VNC setup failed: {err}"),
            );
            return;
        }
    };

    mark_connected(&shared, &runtime, &session_id);
    let _ = client.input(X11Event::FullRefresh).await;

    let mut refresh_tick = tokio::time::interval(Duration::from_millis(33));
    refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event = client.recv_event() => {
                match event {
                    Ok(vnc_event) => {
                        if !apply_vnc_event(&shared, &runtime, &session_id, vnc_event) {
                            break;
                        }
                        while let Ok(Some(more)) = client.poll_event().await {
                            if !apply_vnc_event(&shared, &runtime, &session_id, more) {
                                let _ = client.close().await;
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        mark_disconnected(&shared, &runtime, &session_id, Some(err.to_string()));
                        break;
                    }
                }
            }
            input = input_rx.recv() => {
                let Some(input) = input else {
                    break;
                };
                if view_only && !matches!(input, VncInputEvent::Refresh { .. }) {
                    continue;
                }
                let x11 = to_x11_event(input);
                if client.input(x11).await.is_err() {
                    mark_disconnected(&shared, &runtime, &session_id, None);
                    break;
                }
            }
            _ = refresh_tick.tick() => {
                // An unwatched session stops asking the server for damage:
                // no viewer is polling, so the bandwidth would be wasted.
                let watched = relock(&runtime).polled_recently();
                if watched && client.input(X11Event::Refresh).await.is_err() {
                    mark_disconnected(&shared, &runtime, &session_id, None);
                    break;
                }
            }
        }
    }
    let _ = client.close().await;
}

fn to_x11_event(event: VncInputEvent) -> X11Event {
    match event {
        VncInputEvent::Pointer { x, y, buttons } => X11Event::PointerEvent(ClientMouseEvent {
            position_x: x,
            position_y: y,
            bottons: buttons,
        }),
        VncInputEvent::Key { keycode, down } => {
            X11Event::KeyEvent(ClientKeyEvent { keycode, down })
        }
        VncInputEvent::CopyText(text) => X11Event::CopyText(text),
        VncInputEvent::Refresh { full: true } => X11Event::FullRefresh,
        VncInputEvent::Refresh { full: false } => X11Event::Refresh,
    }
}

fn apply_vnc_event(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    event: VncEvent,
) -> bool {
    match event {
        VncEvent::SetResolution(screen) => {
            {
                let mut rt = relock(runtime);
                rt.resize(screen.width, screen.height);
                rt.status = VncSessionStatus::Connected;
            }
            let mut state = relock(&shared.state);
            if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
                rec.width = screen.width;
                rec.height = screen.height;
                rec.status = VncSessionStatus::Connected;
                state.revision = state.revision.saturating_add(1);
            }
            true
        }
        VncEvent::DesktopUpdate(update) => {
            if let Some(layout) = update.layout {
                {
                    let mut rt = relock(runtime);
                    rt.resize(layout.width, layout.height);
                }
                let mut state = relock(&shared.state);
                if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
                    rec.width = layout.width;
                    rec.height = layout.height;
                    state.revision = state.revision.saturating_add(1);
                }
            }
            true
        }
        VncEvent::RawImage(rect, bytes) => {
            let mut rt = relock(runtime);
            rt.blit_rgba(rect, &bytes);
            true
        }
        VncEvent::Copy(dst, src) => {
            let mut rt = relock(runtime);
            rt.copy_rect(dst, src);
            true
        }
        VncEvent::JpegImage(rect, bytes) => {
            if let Ok(decoded) =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            {
                let rgba = decoded.into_rgba8().into_raw();
                let mut rt = relock(runtime);
                rt.blit_rgba(rect, &rgba);
            }
            true
        }
        VncEvent::Text(text) => {
            let mut rt = relock(runtime);
            rt.clipboard = Some(text);
            rt.revision = rt.revision.saturating_add(1);
            true
        }
        VncEvent::Error(err) => {
            mark_failed(shared, runtime, session_id, err);
            false
        }
        _ => true,
    }
}

fn mark_connected(shared: &Arc<Shared>, runtime: &Arc<Mutex<VncRuntime>>, session_id: &str) {
    {
        let mut rt = relock(runtime);
        rt.status = VncSessionStatus::Connected;
        rt.error = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.status = VncSessionStatus::Connected;
        rec.error = None;
        state.revision = state.revision.saturating_add(1);
    }
}

fn mark_failed(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    message: String,
) {
    {
        let mut rt = relock(runtime);
        rt.status = VncSessionStatus::Failed;
        rt.error = Some(message.clone());
        rt.input_tx = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.status = VncSessionStatus::Failed;
        rec.error = Some(message);
        state.revision = state.revision.saturating_add(1);
    }
}

fn mark_disconnected(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    reason: Option<String>,
) {
    {
        let mut rt = relock(runtime);
        if rt.status == VncSessionStatus::Failed {
            return;
        }
        rt.status = VncSessionStatus::Disconnected;
        rt.error.clone_from(&reason);
        rt.input_tx = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id)
        && rec.status != VncSessionStatus::Failed
    {
        rec.status = VncSessionStatus::Disconnected;
        rec.error = reason;
        state.revision = state.revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    fn runtime() -> VncRuntime {
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        VncRuntime::new(false, input_tx)
    }

    fn pixels(encoded: &str) -> Vec<u8> {
        let compressed = B64.decode(encoded).expect("base64 payload");
        let mut out = Vec::new();
        ZlibDecoder::new(compressed.as_slice())
            .read_to_end(&mut out)
            .expect("zlib payload");
        out
    }

    fn one_pixel(runtime: &mut VncRuntime, x: u16, y: u16, rgba: [u8; 4]) {
        runtime.blit_rgba(
            Rect {
                x,
                y,
                width: 1,
                height: 1,
            },
            &rgba,
        );
    }

    #[test]
    fn a_client_at_the_encoded_revision_takes_the_changed_region_only() {
        let mut rt = runtime();
        rt.resize(4, 4);
        let full = rt.encode_frame("s", 0, true).expect("full frame");
        assert_eq!(full.region, None, "nothing encoded yet: the whole frame");
        assert_eq!(pixels(&full.zlib_rgba).len(), 4 * 4 * 4);
        assert_eq!((full.width, full.height), (4, 4));

        one_pixel(&mut rt, 1, 2, [9, 8, 7, 255]);
        let partial = rt
            .encode_frame("s", full.revision, true)
            .expect("partial frame");
        assert_eq!(
            partial.region,
            Some(VncRegion {
                x: 1,
                y: 2,
                width: 1,
                height: 1
            })
        );
        assert_eq!(pixels(&partial.zlib_rgba), vec![9, 8, 7, 255]);
        assert_eq!(
            (partial.width, partial.height),
            (4, 4),
            "the record keeps describing the whole remote screen"
        );

        // The region was consumed: a poll with no new damage answers with
        // the full frame rather than repeating the one rectangle.
        let next = rt
            .encode_frame("s", partial.revision, true)
            .expect("next frame");
        assert_eq!(next.region, None);
    }

    #[test]
    fn a_client_behind_the_encoded_revision_gets_the_whole_frame() {
        let mut rt = runtime();
        rt.resize(2, 2);
        let full = rt.encode_frame("s", 0, true).expect("full frame");
        one_pixel(&mut rt, 0, 0, [1, 2, 3, 255]);
        // The client never took the last full frame, so a rectangle cut
        // from it would land on the wrong texture.
        let stale = rt
            .encode_frame("s", full.revision.saturating_sub(1), true)
            .expect("stale frame");
        assert_eq!(stale.region, None);
        assert_eq!(pixels(&stale.zlib_rgba).len(), 2 * 2 * 4);
    }

    #[test]
    fn a_client_that_never_asked_for_regions_gets_the_whole_frame() {
        let mut rt = runtime();
        rt.resize(2, 2);
        let full = rt.encode_frame("s", 0, false).expect("full frame");
        one_pixel(&mut rt, 1, 1, [4, 5, 6, 255]);
        let frame = rt
            .encode_frame("s", full.revision, false)
            .expect("legacy frame");
        assert_eq!(frame.region, None);
        assert_eq!(pixels(&frame.zlib_rgba).len(), 2 * 2 * 4);
    }

    #[test]
    fn a_resize_invalidates_incremental_updates() {
        let mut rt = runtime();
        rt.resize(2, 2);
        let full = rt.encode_frame("s", 0, true).expect("full frame");
        rt.resize(3, 3);
        let frame = rt
            .encode_frame("s", full.revision, true)
            .expect("resized frame");
        assert_eq!(frame.region, None, "every pixel changed");
        assert_eq!(pixels(&frame.zlib_rgba).len(), 3 * 3 * 4);
    }

    #[test]
    fn the_server_refresh_follows_the_last_client_poll() {
        let mut rt = runtime();
        assert!(!rt.polled_recently(), "no viewer has asked yet");
        rt.last_poll = Some(Instant::now());
        assert!(
            rt.polled_recently(),
            "a viewer that just polled keeps the refresh running"
        );
        rt.last_poll = Some(
            Instant::now()
                .checked_sub(CLIENT_ACTIVE + Duration::from_secs(1))
                .expect("instant inside the clock range"),
        );
        assert!(
            !rt.polled_recently(),
            "a viewer that stopped asking costs no traffic"
        );
    }
}
