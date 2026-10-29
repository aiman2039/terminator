//! Shared-memory frames (`vnc-shm-v1`): the daemon copies changed
//! rectangles into a memory-mapped file and answers with their list, so
//! pixels never cross the IPC socket.
use super::*;
use std::path::Path;
use terminator_core::vnc::{MAX_FRAME_WAIT_MS, VncShmFrame, VncShmRequest};
use terminator_sys::{FrameRect, SharedFrameWriter};

/// How often a waiting frame request re-checks the revision.
const WAIT_STEP: Duration = Duration::from_millis(2);
const FILE_PREFIX: &str = "vnc-";
const FILE_SUFFIX: &str = ".fb";

pub(crate) struct PublishInput<'a> {
    pub(crate) session: &'a str,
    pub(crate) since: u64,
    pub(crate) request: VncShmRequest,
    pub(crate) runtime_dir: &'a Path,
}

/// Wait until `runtime` has a revision newer than `since`, up to `wait_ms`
/// (capped). Each check counts as the viewer still watching. Returns the
/// guard so the caller encodes under the same lock.
pub(crate) fn wait_for_revision(
    runtime: &Arc<Mutex<VncRuntime>>,
    since: u64,
    wait_ms: u32,
) -> std::sync::MutexGuard<'_, VncRuntime> {
    let wait = Duration::from_millis(u64::from(wait_ms.min(MAX_FRAME_WAIT_MS)));
    let started = Instant::now();
    loop {
        let mut rt = relock(runtime);
        rt.last_poll = Some(Instant::now());
        if rt.revision > since || started.elapsed() >= wait {
            return rt;
        }
        drop(rt);
        thread::sleep(WAIT_STEP);
    }
}

impl VncRuntime {
    /// Copy what changed since `since` into the shared file and describe
    /// it. A viewer that is not exactly at the last published revision, a
    /// new file, or a resize gets the whole frame.
    pub(crate) fn publish_shm(&mut self, input: PublishInput<'_>) -> Result<VncFrame> {
        let PublishInput {
            session,
            since,
            request,
            runtime_dir,
        } = input;
        let mut frame = self.encode_frame_header(session, request.cursor_serial);
        if self.width == 0 || self.height == 0 || self.rgba.is_empty() {
            return Ok(frame);
        }
        if self.shm.as_ref().map(SharedFrameWriter::size) != Some((self.width, self.height)) {
            self.shm = None;
            let path = self.next_shm_path(session, runtime_dir)?;
            self.shm = Some(
                SharedFrameWriter::create(&path, self.width, self.height)
                    .context("Cannot create the shared VNC framebuffer")?,
            );
            self.shm_dirty = dirty::DirtyRects::Full;
        }
        let incremental = since > 0 && since == self.shm_revision;
        let mut regions = self.shm_dirty.take();
        if !incremental {
            regions.clear();
        }
        let rects: Vec<FrameRect> = regions.iter().map(|r| frame_rect(*r)).collect();
        let writer = self
            .shm
            .as_mut()
            .context("Shared VNC framebuffer missing")?;
        let sequence = writer.publish(&self.rgba, &rects)?;
        frame.shm = Some(VncShmFrame {
            path: writer.path().to_owned(),
            sequence,
            regions,
        });
        self.shm_revision = self.revision;
        Ok(frame)
    }

    /// Frame metadata without pixels. The cursor image goes only to a
    /// viewer that has not seen this cursor yet.
    fn encode_frame_header(&self, session: &str, cursor_serial: u64) -> VncFrame {
        VncFrame {
            cursor: (self.cursor_serial > cursor_serial)
                .then(|| self.cursor.clone())
                .flatten(),
            cursor_serial: self.cursor_serial,
            monitors: self.monitors.clone(),
            resize_supported: self.resize_supported,
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
            shm: None,
        }
    }

    fn next_shm_path(&mut self, session: &str, runtime_dir: &Path) -> Result<std::path::PathBuf> {
        ensure!(
            !session.is_empty()
                && session.len() <= 64
                && session
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "Invalid VNC session ID"
        );
        self.shm_files = self.shm_files.saturating_add(1);
        Ok(runtime_dir.join(format!(
            "{FILE_PREFIX}{}-{session}-{}{FILE_SUFFIX}",
            std::process::id(),
            self.shm_files
        )))
    }
}

fn frame_rect(region: VncRegion) -> FrameRect {
    FrameRect {
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
    }
}

/// Remove framebuffer files left by daemons that exited without cleanup.
/// Files of live processes (another generation) are kept.
pub(crate) fn remove_stale_files(runtime_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(runtime_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix(FILE_PREFIX))
            .filter(|n| n.ends_with(FILE_SUFFIX))
            .and_then(|n| n.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !process_alive(pid) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn process_alive(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
    system.process(pid).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_framebuffer_files_of_dead_processes_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir
            .path()
            .join(format!("vnc-{}-s-1.fb", std::process::id()));
        let dead = dir.path().join("vnc-4294967294-s-1.fb");
        let other = dir.path().join("daemon.lock");
        for path in [&live, &dead, &other] {
            std::fs::write(path, b"x").unwrap();
        }
        remove_stale_files(dir.path());
        assert!(live.exists() && other.exists());
        assert!(!dead.exists());
    }
}
