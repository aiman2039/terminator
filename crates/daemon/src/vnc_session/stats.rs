//! Server-side speed of one VNC session: complete updates per second and
//! how long an update takes to arrive, from its first pixels to its end.
//! Servers hold a request until the screen changes, so the time from the
//! request would measure idle time, not the link.
use std::time::{Duration, Instant};
use terminator_core::vnc::VncStats;

const WINDOW: Duration = Duration::from_secs(1);
/// After this long without an update the rate reads as zero.
const STALE: Duration = Duration::from_secs(2);

pub(crate) struct UpdateStats {
    started_at: Option<Instant>,
    update_ms: Option<u32>,
    window_start: Instant,
    window_updates: u32,
    updates_per_second: u16,
    last_update: Option<Instant>,
}

impl Default for UpdateStats {
    fn default() -> Self {
        Self {
            started_at: None,
            update_ms: None,
            window_start: Instant::now(),
            window_updates: 0,
            updates_per_second: 0,
            last_update: None,
        }
    }
}

impl UpdateStats {
    /// Pixels of an update arrived. Only the first one of an update counts.
    pub(crate) fn update_started(&mut self) {
        self.started_at.get_or_insert_with(Instant::now);
    }

    /// The server finished an update.
    pub(crate) fn update_ended(&mut self) {
        let now = Instant::now();
        if let Some(started) = self.started_at.take() {
            let ms = u32::try_from(now.duration_since(started).as_millis()).unwrap_or(u32::MAX);
            // Exponential smoothing so one slow update does not jump the chip.
            self.update_ms = Some(self.update_ms.map_or(ms, |old| smooth(old, ms)));
        }
        self.last_update = Some(now);
        self.window_updates = self.window_updates.saturating_add(1);
        let elapsed = now.duration_since(self.window_start);
        if elapsed >= WINDOW {
            self.updates_per_second = per_second(self.window_updates, elapsed);
            self.window_start = now;
            self.window_updates = 0;
        }
    }

    pub(crate) fn snapshot(&self) -> VncStats {
        let fresh = self.last_update.is_some_and(|t| t.elapsed() < STALE);
        VncStats {
            updates_per_second: if fresh { self.updates_per_second } else { 0 },
            update_ms: self.update_ms.unwrap_or(0),
        }
    }
}

/// Weighted 4:1 toward the old value.
fn smooth(old: u32, new: u32) -> u32 {
    let value = u64::from(old)
        .saturating_mul(4)
        .saturating_add(u64::from(new))
        .checked_div(5)
        .unwrap_or(0);
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `count` events over `elapsed`, rounded, as a per-second rate.
pub(crate) fn per_second(count: u32, elapsed: Duration) -> u16 {
    let millis = elapsed.as_millis().max(1);
    let rate = u128::from(count)
        .saturating_mul(1000)
        .saturating_add(millis.checked_div(2).unwrap_or(0))
        .checked_div(millis)
        .unwrap_or(0);
    u16::try_from(rate).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_time_is_measured_from_the_first_pixels() {
        let mut stats = UpdateStats::default();
        stats.update_started();
        std::thread::sleep(Duration::from_millis(20));
        stats.update_started();
        stats.update_ended();
        assert!(stats.snapshot().update_ms >= 20);
        stats.update_ended();
        assert!(
            stats.snapshot().update_ms >= 20,
            "an empty update is not timed"
        );
    }

    #[test]
    fn a_session_without_updates_reports_no_rate() {
        let stats = UpdateStats::default();
        assert_eq!(stats.snapshot(), VncStats::default());
    }
}
