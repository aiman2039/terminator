use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use flate2::{Compression, write::ZlibEncoder};
use std::{
    io::Write,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use terminator_core::{
    id, now,
    vnc::{VncFrame, VncInputEvent, VncSession, VncSessionStatus},
};
use tokio::sync::mpsc;
use vnc::{
    ClientKeyEvent, ClientMouseEvent, PixelFormat, Rect, VncConnector, VncEncoding, VncEvent,
    X11Event,
};

use crate::server::{Shared, relock};

const MAX_DIMENSION: u16 = 8192;

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
        self.revision = self.revision.saturating_add(1);
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

    pub(crate) fn encode_frame(&self, session: &str) -> Result<VncFrame> {
        let zlib_rgba = if self.width > 0 && self.height > 0 && !self.rgba.is_empty() {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&self.rgba)?;
            B64.encode(encoder.finish()?)
        } else {
            String::new()
        };
        Ok(VncFrame {
            session: session.to_owned(),
            width: self.width,
            height: self.height,
            revision: self.revision,
            status: self.status,
            error: self.error.clone(),
            zlib_rgba,
            clipboard: self.clipboard.clone(),
        })
    }
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
        let record = {
            let mut state = relock(&self.state);
            if !project.is_empty() {
                ensure!(
                    state.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
            }
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

        let (input_tx, input_rx) = mpsc::channel::<VncInputEvent>(256);
        let runtime = Arc::new(Mutex::new(VncRuntime::new(view_only, input_tx)));
        relock(&self.vnc_sessions).insert(sid.clone(), Arc::clone(&runtime));

        let shared_ref = Arc::clone(self);
        let session_id = sid;
        thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    mark_failed(
                        &shared_ref,
                        &runtime,
                        &session_id,
                        "Could not initialize async runtime".into(),
                    );
                    return;
                };
                rt.block_on(run_vnc_session(
                    shared_ref, runtime, session_id, host, port, password, shared, view_only,
                    input_rx,
                ));
            }));
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
            let _ = tx.try_send(event);
        }
        Ok(())
    }

    pub(crate) fn vnc_frame(&self, session: &str, since: u64) -> Result<Option<VncFrame>> {
        let runtime = relock(&self.vnc_sessions)
            .get(session)
            .cloned()
            .context("VNC session is not live")?;
        let rt = relock(&runtime);
        if rt.revision <= since {
            return Ok(None);
        }
        rt.encode_frame(session).map(Some)
    }
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
    mut input_rx: mpsc::Receiver<VncInputEvent>,
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
                if client.input(X11Event::Refresh).await.is_err() {
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
