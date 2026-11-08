//! Read shared-memory frames (`vnc-shm-v1`) published by the daemon.
use super::*;
use anyhow::Context;
use std::path::{Component, Path};
use terminator_core::vnc::VncShmFrame;
use terminator_sys::{FrameRect, SharedFrameReader};

/// Pixels read from a shared frame file.
pub(crate) enum ShmPixels {
    /// The whole frame.
    Full(ColorImage),
    /// Changed rectangles, to apply on top of the current texture.
    Patches(Vec<(VncRegion, ColorImage)>),
    /// The daemon published again while copying; ask for a whole frame.
    Torn,
}

/// Opens a daemon shared frame file, refusing paths outside the runtime
/// directory and files of another size (VNC and remote apps).
pub(crate) fn open_frame(
    path: &Path,
    size: (u16, u16),
    runtime: &Path,
) -> Result<SharedFrameReader> {
    ensure!(
        path.starts_with(runtime) && !path.components().any(|c| matches!(c, Component::ParentDir)),
        "Shared frame is outside the runtime directory"
    );
    let reader = SharedFrameReader::open(path)?;
    ensure!(reader.size() == size, "Shared frame has a different size");
    Ok(reader)
}

/// Copy the published rectangles out of `shm`. Errors mean the file cannot
/// be used at all (outside the runtime directory, wrong size, missing), and
/// the viewer should fall back to compressed frames.
pub(crate) fn read(shm: &VncShmFrame, size: (u16, u16), runtime: &Path) -> Result<ShmPixels> {
    let reader = open_frame(&shm.path, size, runtime)?;
    let rects: Vec<FrameRect> = shm
        .regions
        .iter()
        .map(|r| FrameRect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        })
        .collect();
    let Some(copies) = reader.read(shm.sequence, &rects) else {
        return Ok(ShmPixels::Torn);
    };
    let image = |rect: FrameRect, rgba: &[u8]| {
        ColorImage::from_rgba_unmultiplied(
            [usize::from(rect.width), usize::from(rect.height)],
            rgba,
        )
    };
    if shm.regions.is_empty() {
        let (rect, rgba) = copies.first().context("Empty shared VNC frame")?;
        return Ok(ShmPixels::Full(image(*rect, rgba)));
    }
    Ok(ShmPixels::Patches(
        copies
            .iter()
            .filter(|(rect, _)| rect.width > 0 && rect.height > 0)
            .map(|(rect, rgba)| {
                (
                    VncRegion {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    },
                    image(*rect, rgba),
                )
            })
            .collect(),
    ))
}
