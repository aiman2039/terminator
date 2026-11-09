//! Reading remote window pixels from the daemon's shared-memory files, off
//! the GUI thread: the whole window, or only the regions that changed.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui::ColorImage;
use terminator_core::remote_apps::RemoteAppsState;
use terminator_core::remote_apps::protocol::Rect;
use terminator_sys::{FrameRect, SharedFrameReader};

/// New pixels of one window.
pub(crate) enum Pixels {
    /// The whole window.
    Full(ColorImage),
    /// Changed areas of the window as it was: top-left pixel and image.
    Patches(Vec<([usize; 2], ColorImage)>),
}

/// A state reply with the pixels of the windows that changed.
pub(crate) struct RemoteAppsFrame {
    pub(crate) state: RemoteAppsState,
    pub(crate) images: Vec<(u64, Pixels)>,
    /// A frame changed while it was copied; poll again from the old revision.
    pub(crate) torn: bool,
}

/// One session's open shared-memory files, so a poll does not map a file
/// again. Used off the GUI thread.
pub(crate) type Readers = Arc<Mutex<HashMap<PathBuf, SharedFrameReader>>>;

/// Reads the pixels of windows changed after `since` (off the GUI thread):
/// only their changed regions when the daemon sent them.
pub(crate) fn decode(
    state: RemoteAppsState,
    since: u64,
    (runtime, readers): (&Path, &Readers),
) -> Result<RemoteAppsFrame> {
    let mut readers = readers
        .lock()
        .map_err(|_| anyhow::anyhow!("Remote app readers are poisoned"))?;
    readers.retain(|path, _| state.windows.iter().any(|w| w.shm.as_ref() == Some(path)));
    let mut images = Vec::new();
    let mut torn = false;
    for window in state.windows.iter().filter(|w| w.revision > since) {
        let Some(path) = &window.shm else { continue };
        let size = (window.width, window.height);
        let reader = match readers.entry(path.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let Ok(reader) = crate::vnc_ui::open_frame(path, size, runtime) else {
                    // The daemon moved the window to a new file.
                    torn = true;
                    continue;
                };
                entry.insert(reader)
            }
        };
        match read_window(reader, size, window.sequence, &window.regions) {
            Some(pixels) => images.push((window.id.0, pixels)),
            None => torn = true,
        }
    }
    Ok(RemoteAppsFrame {
        state,
        images,
        torn,
    })
}

/// `None` for a torn copy or a region outside the window.
fn read_window(
    reader: &SharedFrameReader,
    size: (u16, u16),
    sequence: u64,
    regions: &[Rect],
) -> Option<Pixels> {
    if reader.size() != size {
        return None;
    }
    let rects: Vec<FrameRect> = regions
        .iter()
        .map(|r| {
            Some(FrameRect {
                x: u16::try_from(r.x).ok()?,
                y: u16::try_from(r.y).ok()?,
                width: u16::try_from(r.width).ok()?,
                height: u16::try_from(r.height).ok()?,
            })
        })
        .collect::<Option<_>>()?;
    let copies = reader.read(sequence, &rects)?;
    let image = |rect: &FrameRect, rgba: &[u8]| {
        ColorImage::from_rgba_premultiplied(
            [usize::from(rect.width), usize::from(rect.height)],
            rgba,
        )
    };
    if rects.is_empty() {
        let (rect, rgba) = copies.first()?;
        return Some(Pixels::Full(image(rect, rgba)));
    }
    Some(Pixels::Patches(
        copies
            .iter()
            .map(|(rect, rgba)| {
                (
                    [usize::from(rect.x), usize::from(rect.y)],
                    image(rect, rgba),
                )
            })
            .collect(),
    ))
}
