use super::*;
pub(crate) struct VncDecodedFrame {
    pub(crate) session: String,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    pub(crate) image: Option<ColorImage>,
    /// Set when `image` covers only this rectangle of the frame.
    pub(crate) region: Option<VncRegion>,
    /// Changed rectangles of a shared-memory frame, applied in order.
    pub(crate) patches: Vec<(VncRegion, ColorImage)>,
    pub(crate) cursor_serial: u64,
    /// The shared-memory file was unusable; use compressed frames.
    pub(crate) shm_failed: bool,
    pub(crate) clipboard: Option<String>,
    pub(crate) cursor: Option<(terminator_core::vnc::VncCursor, Option<ColorImage>)>,
    pub(crate) monitors: Vec<terminator_core::vnc::VncMonitor>,
    pub(crate) resize_supported: bool,
    /// Server speed from a shared-memory frame.
    pub(crate) stats: Option<terminator_core::vnc::VncStats>,
}

/// Decode a frame off the GUI thread. `runtime` is the daemon runtime
/// directory; shared-memory frame files must be inside it.
pub(crate) fn decode_frame(frame: VncFrame, runtime: &std::path::Path) -> Result<VncDecodedFrame> {
    let mut patches = Vec::new();
    let mut shm_failed = false;
    let mut image = match &frame.shm {
        Some(shm) => match shm::read(shm, (frame.width, frame.height), runtime) {
            Ok(shm::ShmPixels::Full(image)) => Some(image),
            Ok(shm::ShmPixels::Patches(rects)) => {
                patches = rects;
                None
            }
            Ok(shm::ShmPixels::Torn) => None,
            Err(_) => {
                shm_failed = true;
                None
            }
        },
        None => None,
    };
    if frame.shm.is_none() {
        image = decode_pixels(&frame)?;
    }
    let cursor = frame
        .cursor
        .map(|cursor| -> Result<_> {
            ensure!(
                cursor.width <= 256 && cursor.height <= 256,
                "Remote cursor exceeds 256 pixels"
            );
            let bytes = B64.decode(&cursor.rgba)?;
            let size = [usize::from(cursor.width), usize::from(cursor.height)];
            ensure!(
                bytes.len() == size[0].saturating_mul(size[1]).saturating_mul(4),
                "Invalid cursor image"
            );
            let image = (cursor.width > 0 && cursor.height > 0)
                .then(|| ColorImage::from_rgba_unmultiplied(size, &bytes));
            Ok((cursor, image))
        })
        .transpose()?;
    Ok(VncDecodedFrame {
        stats: frame.shm.as_ref().map(|shm| shm.stats),
        cursor,
        monitors: frame.monitors,
        resize_supported: frame.resize_supported,
        session: frame.session,
        width: frame.width,
        height: frame.height,
        revision: frame.revision,
        status: frame.status,
        error: frame.error,
        image,
        region: frame.region,
        patches,
        cursor_serial: frame.cursor_serial,
        shm_failed,
        clipboard: frame.clipboard,
    })
}

/// Zlib + base64 pixels of a compressed frame.
fn decode_pixels(frame: &VncFrame) -> Result<Option<ColorImage>> {
    // A partial frame carries the changed rectangle only; the record's
    // width/height still describe the whole remote screen.
    let (pixels_w, pixels_h) = match frame.region {
        Some(region) => (region.width, region.height),
        None => (frame.width, frame.height),
    };
    let image = if pixels_w > 0 && pixels_h > 0 && !frame.zlib_rgba.is_empty() {
        let compressed = B64.decode(&frame.zlib_rgba)?;
        let expected = usize::from(pixels_w)
            .saturating_mul(usize::from(pixels_h))
            .saturating_mul(4);
        ensure!(
            expected <= 128 * 1024 * 1024,
            "VNC frame exceeds 128 MiB limit"
        );
        let mut rgba = Vec::with_capacity(expected);
        ZlibDecoder::new(compressed.as_slice())
            .take(u64::try_from(expected).unwrap_or(u64::MAX))
            .read_to_end(&mut rgba)?;
        if rgba.len() == expected {
            Some(ColorImage::from_rgba_unmultiplied(
                [usize::from(pixels_w), usize::from(pixels_h)],
                &rgba,
            ))
        } else {
            None
        }
    } else {
        None
    };
    Ok(image)
}

impl App {
    pub(crate) fn apply_vnc_frame(&mut self, ctx: &egui::Context, decoded: VncDecodedFrame) {
        let entry = self.vnc_views.entry(decoded.session.clone()).or_default();
        entry.fetching = false;
        entry.cursor_serial = decoded.cursor_serial;
        entry.shm_failed |= decoded.shm_failed;
        entry.monitors = decoded.monitors;
        entry.resize_supported = decoded.resize_supported;
        if decoded.stats.is_some() {
            entry.speed.server = decoded.stats;
        }
        if let Some((cursor, image)) = decoded.cursor
            && (entry.cursor_signature.as_ref() != Some(&cursor.rgba)
                || entry.cursor_hotspot != [cursor.x, cursor.y]
                || image.as_ref().map(|i| i.size)
                    != entry.cursor_texture.as_ref().map(TextureHandle::size))
        {
            entry.cursor_signature = Some(cursor.rgba);
            entry.cursor_hotspot = [cursor.x, cursor.y];
            entry.cursor_texture = image.map(|image| {
                ctx.load_texture(
                    format!("vnc-cursor:{}", decoded.session),
                    image,
                    egui::TextureOptions::NEAREST,
                )
            });
        }
        if decoded.status != VncSessionStatus::Connected {
            entry.held_keys.clear();
            entry.modifier_bits = 0;
            entry.focused = false;
        }

        entry.width = decoded.width;
        entry.height = decoded.height;
        entry.revision = decoded.revision;
        entry.status = decoded.status;
        entry.error = decoded.error;
        let frame_size = [usize::from(decoded.width), usize::from(decoded.height)];
        let mut applied = apply_patches(entry, decoded.patches, frame_size);
        if let Some(image) = decoded.image {
            // A partial frame is valid only on top of the texture the daemon
            // cut it from. Anything else (no texture yet, a resize, a
            // dropped frame) falls through to a full request.
            if let Some(region) = decoded.region {
                let in_bounds = usize::from(region.x).saturating_add(usize::from(region.width))
                    <= frame_size[0]
                    && usize::from(region.y).saturating_add(usize::from(region.height))
                        <= frame_size[1];
                if in_bounds
                    && let Some(texture) = entry.texture.as_mut()
                    && texture.size() == frame_size
                {
                    texture.set_partial(
                        [usize::from(region.x), usize::from(region.y)],
                        image,
                        entry.scale.filter(),
                    );
                    applied = true;
                }
            } else {
                entry.texture = Some(ctx.load_texture(
                    format!("vnc:{}", decoded.session),
                    image,
                    entry.scale.filter(),
                ));
                applied = true;
            }
        }
        if applied {
            entry.speed.frame_shown();
        } else {
            entry.force_full = true;
        }
        if let Some(clipboard) = decoded.clipboard
            && entry.remote_clipboard.as_ref() != Some(&clipboard)
        {
            entry.remote_clipboard = Some(clipboard.clone());
            ctx.copy_text(clipboard);
        }
    }
}

/// Apply shared-memory rectangles on top of a texture of the same size.
/// Returns false when there is nothing to apply or no matching texture.
fn apply_patches(
    entry: &mut VncViewState,
    patches: Vec<(VncRegion, ColorImage)>,
    frame_size: [usize; 2],
) -> bool {
    if patches.is_empty() {
        return false;
    }
    let Some(texture) = entry.texture.as_mut().filter(|t| t.size() == frame_size) else {
        return false;
    };
    for (region, image) in patches {
        let fits = usize::from(region.x).saturating_add(usize::from(region.width)) <= frame_size[0]
            && usize::from(region.y).saturating_add(usize::from(region.height)) <= frame_size[1];
        if !fits {
            return false;
        }
        texture.set_partial(
            [usize::from(region.x), usize::from(region.y)],
            image,
            entry.scale.filter(),
        );
    }
    true
}
