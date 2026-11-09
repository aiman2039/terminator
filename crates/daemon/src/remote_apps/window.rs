//! One remote window in the daemon: its metadata, its shared-memory file,
//! and a short damage history to answer "what changed since revision N".

use std::collections::VecDeque;
use std::path::PathBuf;

use terminator_core::remote_apps::protocol::Rect;
use terminator_core::remote_apps::{RemoteWindow, frame_rect};
use terminator_sys::{FrameRect, SharedFrameWriter};

/// Damage kept per window.
const DAMAGE_HISTORY: usize = 32;

pub(super) struct Window {
    pub(super) meta: RemoteWindow,
    /// Holds the window's pixels; dropping it removes its file.
    writer: Option<SharedFrameWriter>,
    /// (session revision, changed area), oldest first.
    damage: VecDeque<(u64, Rect)>,
    /// Every change after this revision is in `damage`.
    damage_from: u64,
}

impl Window {
    pub(super) fn new(meta: RemoteWindow) -> Self {
        let damage_from = meta.revision;
        Self {
            meta,
            writer: None,
            damage: VecDeque::new(),
            damage_from,
        }
    }

    /// The size of the shared-memory file, if there is one.
    pub(super) fn size(&self) -> Option<(u16, u16)> {
        self.writer.as_ref().map(SharedFrameWriter::size)
    }

    /// Writes `damage` (with its RGBA) into the shared-memory file at
    /// `revision`. `new_file` moves the window to a new file of `size`
    /// first (the damage is then the whole window). `false` when nothing was
    /// published.
    pub(super) fn publish(
        &mut self,
        (revision, size, new_file): (u64, (u16, u16), Option<PathBuf>),
        damage: &[Rect],
        rgba: &[Vec<u8>],
    ) -> bool {
        let (width, height) = size;
        let resized = new_file.is_some();
        if let Some(path) = new_file {
            self.writer = None;
            self.writer = SharedFrameWriter::create(&path, width, height).ok();
            self.damage.clear();
            self.damage_from = revision;
        }
        let Some(rects) = damage
            .iter()
            .zip(rgba)
            .map(|(rect, src)| Some((frame_rect(*rect)?, src.as_slice())))
            .collect::<Option<Vec<(FrameRect, &[u8])>>>()
        else {
            return false;
        };
        let Some(writer) = self.writer.as_mut() else {
            return false;
        };
        let Ok(sequence) = writer.publish_packed(&rects) else {
            return false;
        };
        if !resized {
            for rect in damage {
                self.damage.push_back((revision, *rect));
            }
            while self.damage.len() > DAMAGE_HISTORY {
                if let Some((at, _)) = self.damage.pop_front() {
                    self.damage_from = at;
                }
            }
        }
        self.meta.width = width;
        self.meta.height = height;
        self.meta.shm = Some(writer.path().to_path_buf());
        self.meta.sequence = sequence;
        self.meta.revision = revision;
        true
    }

    /// The one area that changed after `since`, when the history covers
    /// that; `None` means the viewer must read the whole window.
    pub(super) fn changed_since(&self, since: u64) -> Option<Rect> {
        if since < self.damage_from || self.meta.revision <= since {
            return None;
        }
        self.damage
            .iter()
            .filter(|(at, _)| *at > since)
            .map(|(_, rect)| *rect)
            .reduce(terminator_core::remote_apps::union)
    }
}
