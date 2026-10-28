use super::*;
struct ApplyResolutionInput {
    width: u16,
    height: u16,
}
fn apply_resolution(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    input: ApplyResolutionInput,
) -> bool {
    let ApplyResolutionInput { width, height } = input;
    {
        let mut rt = relock(runtime);
        rt.resize(width, height);
        if rt.status == VncSessionStatus::Failed {
            return false;
        }
        rt.status = VncSessionStatus::Connected;
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.width = width;
        rec.height = height;
        rec.status = VncSessionStatus::Connected;
        state.revision = state.revision.saturating_add(1);
    }
    true
}
fn apply_layout(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    layout: vnc::DesktopLayout,
) -> bool {
    {
        let mut rt = relock(runtime);
        rt.resize(layout.width, layout.height);
        if rt.status == VncSessionStatus::Failed {
            return false;
        }
        rt.monitors = layout
            .screens
            .iter()
            .map(|s| terminator_core::vnc::VncMonitor {
                id: s.id,
                x: s.x,
                y: s.y,
                width: s.width,
                height: s.height,
            })
            .collect();
        rt.resize_supported = true;
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.width = layout.width;
        rec.height = layout.height;
        state.revision = state.revision.saturating_add(1);
    }
    true
}
fn apply_jpeg(runtime: &Arc<Mutex<VncRuntime>>, rect: vnc::Rect, bytes: Vec<u8>) -> bool {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(u32::from(MAX_DIMENSION));
    limits.max_image_height = Some(u32::from(MAX_DIMENSION));
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    if let Ok(decoded) = reader.decode() {
        if decoded.width() != u32::from(rect.width) || decoded.height() != u32::from(rect.height) {
            return false;
        }
        relock(runtime).blit_rgba(rect, &decoded.into_rgba8().into_raw());
        true
    } else {
        false
    }
}
fn apply_cursor(runtime: &Arc<Mutex<VncRuntime>>, rect: vnc::Rect, pixels: Vec<u8>) -> bool {
    if rect.width <= 256
        && rect.height <= 256
        && pixels.len()
            == usize::from(rect.width)
                .saturating_mul(usize::from(rect.height))
                .saturating_mul(4)
    {
        let mut rt = relock(runtime);
        rt.cursor = Some(terminator_core::vnc::VncCursor {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            rgba: B64.encode(pixels),
        });
        rt.revision = rt.revision.saturating_add(1);
    }
    true
}
pub(super) fn apply_vnc_event(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
    event: VncEvent,
) -> bool {
    match event {
        VncEvent::SetResolution(screen) => apply_resolution(
            shared,
            runtime,
            session_id,
            ApplyResolutionInput {
                width: screen.width,
                height: screen.height,
            },
        ),
        VncEvent::DesktopUpdate(update) => match update.layout {
            Some(layout) => apply_layout(shared, runtime, session_id, layout),
            None => true,
        },
        VncEvent::RawImage(rect, bytes) => {
            relock(runtime).blit_rgba(rect, &bytes);
            true
        }
        VncEvent::Copy(dst, src) => {
            relock(runtime).copy_rect(dst, src);
            true
        }
        VncEvent::JpegImage(rect, bytes) => apply_jpeg(runtime, rect, bytes),
        VncEvent::SetCursor(rect, pixels) => apply_cursor(runtime, rect, pixels),
        VncEvent::Text(text) => {
            let mut rt = relock(runtime);
            if rt.options.clipboard.receive() && text.len() <= terminator_core::vnc::MAX_CLIPBOARD {
                rt.clipboard = Some(text);
            }
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
