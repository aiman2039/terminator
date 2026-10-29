//! Shared-memory RGBA framebuffer between the daemon (writer) and the GUI
//! (reader), so frames do not cross the IPC socket.
//!
//! Layout: a 64-byte header, then `width * height * 4` RGBA bytes.
//! Header: magic (8), sequence (8, atomic), width (4), height (4), reserved.
//!
//! The sequence is a seqlock: odd while the writer copies pixels, even when
//! the pixels are consistent. A reader copies, then checks that the sequence
//! still has the value the daemon reported; any change means the copy may be
//! torn and must be discarded.
//!
//! The writer never truncates or resizes a published file: a new size gets a
//! new file. A reader's mapping therefore never extends past end-of-file, so
//! reads cannot fault (`SIGBUS`), even after the writer unlinks the file.
use memmap2::{Mmap, MmapMut};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAGIC: &[u8; 8] = b"TRMVNC01";
const HEADER: usize = 64;
const SEQUENCE: usize = 8;
const WIDTH: usize = 16;
const HEIGHT: usize = 20;

/// A rectangle in framebuffer pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

fn pixel_bytes(width: u16, height: u16) -> usize {
    usize::from(width)
        .saturating_mul(usize::from(height))
        .saturating_mul(4)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// The header sequence word for loads. The mapping is page-aligned, so
/// offset 8 is 8-byte aligned, and the borrow keeps the mapping alive.
fn sequence(map: &[u8]) -> Option<&AtomicU64> {
    let word = map.get(SEQUENCE..SEQUENCE.saturating_add(8))?;
    atomic_at(word.as_ptr().cast_mut())
}

/// The header sequence word for the writer, derived from the mutable map.
fn sequence_mut(map: &mut MmapMut) -> Option<&AtomicU64> {
    let word = map.get_mut(SEQUENCE..SEQUENCE.saturating_add(8))?;
    atomic_at(word.as_mut_ptr())
}

#[expect(
    clippy::cast_ptr_alignment,
    reason = "the alignment is checked before the cast"
)]
fn atomic_at<'a>(ptr: *mut u8) -> Option<&'a AtomicU64> {
    if ptr.align_offset(std::mem::align_of::<AtomicU64>()) != 0 {
        return None;
    }
    let ptr = ptr.cast::<u64>();
    // SAFETY: callers pass 8 in-bounds, aligned bytes of a live mapping
    // borrowed for the returned lifetime. Every access to this word, in
    // this process and the peer, goes through atomics; readers only load.
    Some(unsafe { AtomicU64::from_ptr(ptr) })
}

/// Copy `rect` rows between a full-frame buffer and a packed rectangle.
fn rect_rows(width: u16, rect: FrameRect) -> impl Iterator<Item = (usize, usize, usize)> {
    let stride = usize::from(width).saturating_mul(4);
    let row_bytes = usize::from(rect.width).saturating_mul(4);
    let x_bytes = usize::from(rect.x).saturating_mul(4);
    (0..usize::from(rect.height)).map(move |row| {
        let frame = usize::from(rect.y)
            .saturating_add(row)
            .saturating_mul(stride)
            .saturating_add(x_bytes);
        (frame, row.saturating_mul(row_bytes), row_bytes)
    })
}

fn in_bounds(width: u16, height: u16, rect: FrameRect) -> bool {
    u32::from(rect.x).saturating_add(u32::from(rect.width)) <= u32::from(width)
        && u32::from(rect.y).saturating_add(u32::from(rect.height)) <= u32::from(height)
}

/// Daemon side: owns the file and removes it on drop.
pub struct SharedFrameWriter {
    path: PathBuf,
    map: MmapMut,
    width: u16,
    height: u16,
}

impl SharedFrameWriter {
    /// Create a new, private (0600) file at `path`. Fails if it exists.
    pub fn create(path: &Path, width: u16, height: u16) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file: File = options.open(path)?;
        let len = HEADER.saturating_add(pixel_bytes(width, height));
        file.set_len(u64::try_from(len).map_err(|_| invalid("frame too large"))?)?;
        // SAFETY: the file was just created with `create_new` in the private
        // runtime directory and is never truncated while mapped. Concurrent
        // readers only read; the sequence word is accessed atomically.
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        let header = map.get_mut(..HEADER).ok_or_else(|| invalid("short map"))?;
        header.fill(0);
        if let Some(magic) = header.get_mut(..8) {
            magic.copy_from_slice(MAGIC);
        }
        if let Some(w) = header.get_mut(WIDTH..WIDTH.saturating_add(4)) {
            w.copy_from_slice(&u32::from(width).to_le_bytes());
        }
        if let Some(h) = header.get_mut(HEIGHT..HEIGHT.saturating_add(4)) {
            h.copy_from_slice(&u32::from(height).to_le_bytes());
        }
        Ok(Self {
            path: path.to_owned(),
            map,
            width,
            height,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Copy `rects` (or the whole frame when empty) from `frame`, a full
    /// `width * height * 4` RGBA buffer. Returns the new even sequence.
    pub fn publish(&mut self, frame: &[u8], rects: &[FrameRect]) -> io::Result<u64> {
        if frame.len() != pixel_bytes(self.width, self.height) {
            return Err(invalid("frame size does not match the shared buffer"));
        }
        let start = sequence_mut(&mut self.map)
            .ok_or_else(|| invalid("short map"))?
            .fetch_add(1, Ordering::AcqRel);
        let (width, height) = (self.width, self.height);
        let pixels = self
            .map
            .get_mut(HEADER..)
            .ok_or_else(|| invalid("short map"))?;
        if rects.is_empty() {
            if let Some(dst) = pixels.get_mut(..frame.len()) {
                dst.copy_from_slice(frame);
            }
        } else {
            for rect in rects.iter().filter(|r| in_bounds(width, height, **r)) {
                for (offset, _, len) in rect_rows(width, *rect) {
                    let range = offset..offset.saturating_add(len);
                    if let (Some(dst), Some(src)) =
                        (pixels.get_mut(range.clone()), frame.get(range))
                    {
                        dst.copy_from_slice(src);
                    }
                }
            }
        }
        let end = start.wrapping_add(2) & !1;
        sequence_mut(&mut self.map)
            .ok_or_else(|| invalid("short map"))?
            .store(end, Ordering::Release);
        Ok(end)
    }
}

impl Drop for SharedFrameWriter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// GUI side: a read-only mapping of a writer's file.
pub struct SharedFrameReader {
    map: Mmap,
    width: u16,
    height: u16,
}

impl SharedFrameReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: the daemon never truncates a published file, so the
        // mapping stays backed. Pixel bytes may change underneath; `read`
        // detects that through the seqlock and discards the copy.
        let map = unsafe { Mmap::map(&file)? };
        let header = map
            .get(..HEADER)
            .ok_or_else(|| invalid("short frame file"))?;
        if header.get(..8) != Some(MAGIC.as_slice()) {
            return Err(invalid("not a frame file"));
        }
        let word = |at: usize| {
            header
                .get(at..at.saturating_add(4))
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(u32::from_le_bytes)
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| invalid("bad frame header"))
        };
        let (width, height) = (word(WIDTH)?, word(HEIGHT)?);
        if map.len() < HEADER.saturating_add(pixel_bytes(width, height)) {
            return Err(invalid("short frame file"));
        }
        Ok(Self { map, width, height })
    }

    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Packed RGBA for each rect (the whole frame when `rects` is empty),
    /// or `None` when the sequence differs from `expected`: the writer
    /// published again, and the copy may be torn.
    #[must_use]
    pub fn read(&self, expected: u64, rects: &[FrameRect]) -> Option<Vec<(FrameRect, Vec<u8>)>> {
        let seq = sequence(&self.map)?;
        if seq.load(Ordering::Acquire) != expected || !expected.is_multiple_of(2) {
            return None;
        }
        let pixels = self.map.get(HEADER..)?;
        let whole = [FrameRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }];
        let rects = if rects.is_empty() { &whole[..] } else { rects };
        let mut out = Vec::with_capacity(rects.len());
        for rect in rects {
            if !in_bounds(self.width, self.height, *rect) {
                return None;
            }
            let mut packed = vec![0u8; pixel_bytes(rect.width, rect.height)];
            for (offset, packed_offset, len) in rect_rows(self.width, *rect) {
                let src = pixels.get(offset..offset.saturating_add(len))?;
                packed
                    .get_mut(packed_offset..packed_offset.saturating_add(len))?
                    .copy_from_slice(src);
            }
            out.push((*rect, packed));
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (seq.load(Ordering::Acquire) == expected).then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u16, height: u16, fill: u8) -> Vec<u8> {
        vec![fill; pixel_bytes(width, height)]
    }

    #[test]
    fn a_reader_sees_published_rects_and_rejects_a_newer_sequence() {
        let dir = std::env::temp_dir().join(format!("trm-shm-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("rects.fb");
        let mut writer = SharedFrameWriter::create(&path, 4, 3).unwrap();
        let first = writer.publish(&frame(4, 3, 7), &[]).unwrap();
        let reader = SharedFrameReader::open(&path).unwrap();
        assert_eq!(reader.size(), (4, 3));
        let whole = reader.read(first, &[]).unwrap();
        assert!(whole[0].1.iter().all(|b| *b == 7));

        let mut next = frame(4, 3, 7);
        let rect = FrameRect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        for (offset, _, len) in rect_rows(4, rect) {
            next[offset..offset + len].fill(9);
        }
        let second = writer.publish(&next, &[rect]).unwrap();
        assert!(second > first && second.is_multiple_of(2));
        assert!(reader.read(first, &[rect]).is_none(), "stale sequence");
        let patch = reader.read(second, &[rect]).unwrap();
        assert_eq!(patch[0].0, rect);
        assert!(patch[0].1.iter().all(|b| *b == 9));
        assert!(
            reader
                .read(
                    second,
                    &[FrameRect {
                        x: 3,
                        y: 0,
                        width: 2,
                        height: 1
                    }]
                )
                .is_none()
        );
    }

    #[test]
    fn dropping_the_writer_removes_the_file_but_keeps_mappings_readable() {
        let dir = std::env::temp_dir().join(format!("trm-shm-drop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("drop.fb");
        let mut writer = SharedFrameWriter::create(&path, 2, 2).unwrap();
        assert!(
            SharedFrameWriter::create(&path, 2, 2).is_err(),
            "never reuses a file"
        );
        let seq = writer.publish(&frame(2, 2, 5), &[]).unwrap();
        let reader = SharedFrameReader::open(&path).unwrap();
        drop(writer);
        assert!(!path.exists());
        assert!(reader.read(seq, &[]).is_some());
    }
}
