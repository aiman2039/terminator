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
    pub(crate) clipboard: Option<String>,
    pub(crate) cursor: Option<(terminator_core::vnc::VncCursor, Option<ColorImage>)>,
    pub(crate) monitors: Vec<terminator_core::vnc::VncMonitor>,
    pub(crate) resize_supported: bool,
}

pub(crate) fn decode_frame(frame: VncFrame) -> Result<VncDecodedFrame> {
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
        clipboard: frame.clipboard,
    })
}
