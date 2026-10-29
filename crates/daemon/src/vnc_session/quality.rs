use std::time::{Duration, Instant};
use terminator_core::vnc::Quality;
use vnc::VncEncoding;

/// Encodings for a quality setting. `slow_link` selects the JPEG set that
/// Automatic quality uses while updates are slow.
pub(super) fn encodings(quality: Quality, slow_link: bool) -> Vec<VncEncoding> {
    let mut list = match quality {
        Quality::Sharp => vec![VncEncoding::Zrle, VncEncoding::CopyRect, VncEncoding::Raw],
        Quality::Auto if !slow_link => vec![
            VncEncoding::Tight,
            VncEncoding::Zrle,
            VncEncoding::CopyRect,
            VncEncoding::Raw,
        ],
        Quality::LowBandwidth | Quality::Auto => vec![
            VncEncoding::Tight,
            VncEncoding::QualityLow,
            VncEncoding::CompressHigh,
            VncEncoding::Zrle,
            VncEncoding::CopyRect,
            VncEncoding::Raw,
        ],
    };
    // The cursor shape is always requested: the server then leaves the
    // pointer out of the framebuffer and the viewer draws it locally at the
    // local mouse position, so it never lags behind the frame stream.
    list.extend([
        VncEncoding::DesktopSizePseudo,
        VncEncoding::ExtendedDesktopSizePseudo,
        VncEncoding::CursorPseudo,
    ]);
    list
}

/// Shortest time between two update requests. Requests are pipelined, so
/// this caps the frame rate (about 60 per second) rather than adding delay.
/// `recent_bytes` is the last legacy encode; shared-memory frames leave it 0.
pub(super) fn refresh_ms(quality: Quality, recent_bytes: usize) -> u64 {
    match quality {
        Quality::LowBandwidth => 100,
        Quality::Sharp => 16,
        Quality::Auto => {
            if recent_bytes > 2 * 1024 * 1024 {
                66
            } else {
                16
            }
        }
    }
}

/// Updates slower than this (smoothed) mean the link is the bottleneck.
const SLOW_MS: u32 = 80;
/// Updates faster than this mean lossless encodings keep up again.
const FAST_MS: u32 = 30;
/// A condition must hold this long before Automatic switches, so one
/// large update does not flip encodings back and forth.
const HOLD: Duration = Duration::from_secs(3);

/// Automatic quality: JPEG while updates are slow, lossless when fast.
#[derive(Default)]
pub(super) struct Adaptive {
    slow_link: bool,
    candidate_since: Option<Instant>,
}

impl Adaptive {
    /// Feed the smoothed update time. Returns new encodings when Automatic
    /// should switch; other quality settings never switch.
    pub(super) fn check(&mut self, quality: Quality, update_ms: u32) -> Option<Vec<VncEncoding>> {
        if quality != Quality::Auto || update_ms == 0 {
            self.candidate_since = None;
            return None;
        }
        let wants_switch = if self.slow_link {
            update_ms < FAST_MS
        } else {
            update_ms > SLOW_MS
        };
        if !wants_switch {
            self.candidate_since = None;
            return None;
        }
        let since = *self.candidate_since.get_or_insert_with(Instant::now);
        if since.elapsed() < HOLD {
            return None;
        }
        self.slow_link = !self.slow_link;
        self.candidate_since = None;
        Some(encodings(quality, self.slow_link))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_quality_switches_only_after_slow_updates_persist() {
        let mut adaptive = Adaptive::default();
        assert!(
            adaptive.check(Quality::Auto, 200).is_none(),
            "one slow sample waits"
        );
        adaptive.candidate_since = Instant::now().checked_sub(HOLD);
        let slow = adaptive
            .check(Quality::Auto, 200)
            .expect("persistently slow switches");
        assert!(slow.contains(&VncEncoding::QualityLow));
        assert!(
            adaptive.check(Quality::Auto, 50).is_none(),
            "between thresholds stays"
        );
        assert!(
            adaptive.check(Quality::Sharp, 500).is_none(),
            "fixed quality never switches"
        );
    }
}
