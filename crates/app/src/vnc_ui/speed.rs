//! The toolbar speed indicator: frames the viewer shows per second, and
//! (from `vnc-shm-v1` daemons) how fast the server answers update requests.
use super::*;
use terminator_core::vnc::VncStats;

const WINDOW: Duration = Duration::from_secs(1);
/// After this long without a new frame the rate reads as zero.
const STALE: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct ViewSpeed {
    window_start: Option<Instant>,
    window_frames: u32,
    fps: u16,
    last_frame: Option<Instant>,
    /// Latest server-side numbers; `None` from older daemons.
    pub(crate) server: Option<VncStats>,
}

impl ViewSpeed {
    /// A frame changed the texture.
    pub(crate) fn frame_shown(&mut self) {
        let now = Instant::now();
        self.last_frame = Some(now);
        let start = *self.window_start.get_or_insert(now);
        self.window_frames = self.window_frames.saturating_add(1);
        let elapsed = now.duration_since(start);
        if elapsed >= WINDOW {
            let millis = elapsed.as_millis().max(1);
            let rate = u128::from(self.window_frames)
                .saturating_mul(1000)
                .saturating_add(millis.checked_div(2).unwrap_or(0))
                .checked_div(millis)
                .unwrap_or(0);
            self.fps = u16::try_from(rate).unwrap_or(u16::MAX);
            self.window_start = Some(now);
            self.window_frames = 0;
        }
    }

    pub(crate) fn fps(&self) -> u16 {
        if self.last_frame.is_some_and(|t| t.elapsed() < STALE) {
            self.fps
        } else {
            0
        }
    }

    /// Short chip text, for example `58 fps · 14 ms`.
    pub(crate) fn label(&self) -> String {
        match self.server.filter(|s| s.update_ms > 0) {
            Some(server) => format!("{} fps · {} ms", self.fps(), server.update_ms),
            None => format!("{} fps", self.fps()),
        }
    }

    pub(crate) fn detail(&self) -> String {
        let mut text = format!("Viewer: {} frames per second shown.", self.fps());
        if let Some(server) = self.server {
            text.push_str(&format!(
                "\nServer: {} updates per second; an update takes {} ms to arrive and decode.",
                server.updates_per_second, server.update_ms
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_shows_server_time_only_when_known() {
        let mut speed = ViewSpeed::default();
        assert_eq!(speed.label(), "0 fps");
        speed.server = Some(VncStats {
            updates_per_second: 30,
            update_ms: 14,
        });
        assert_eq!(speed.label(), "0 fps · 14 ms");
    }
}
