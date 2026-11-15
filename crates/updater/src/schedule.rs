//! App-owned minute probes, with one background offer per version per launch.
use std::{collections::HashSet, time::Duration};

#[derive(Debug, PartialEq)]
pub(super) enum Action {
    None,
    Probe,
    Offer,
}

#[derive(Default)]
pub(super) struct UpdateSchedule {
    next: Duration,
    probing: bool,
    pending: Option<String>,
    offered: HashSet<String>,
    /// When the running probe started.
    probed_at: Duration,
    /// Probes in a row that could not reach the feed.
    unreachable_streak: u32,
    /// The running probe could not reach the feed.
    unreachable: bool,
}
impl UpdateSchedule {
    pub fn next_action(&mut self, now: Duration, enabled: bool, available: bool) -> Action {
        if !enabled {
            self.pending = None;
            return Action::None;
        }
        // Sparkle not ready / session in progress: retry without consuming the minute.
        if !available || self.probing {
            return Action::None;
        }
        if let Some(version) = self.pending.take()
            && self.offered.insert(version)
        {
            return Action::Offer;
        }
        if now < self.next {
            return Action::None;
        }
        if let Some(next) = now.checked_add(Duration::from_mins(1)) {
            self.next = next;
        }
        self.probed_at = now;
        self.probing = true;
        Action::Probe
    }
    pub fn found(&mut self, version: String) {
        if self.probing {
            self.pending = Some(version);
        }
    }
    /// The running probe failed (could not reach or read the feed; "no
    /// update" is not a failure).
    pub fn unreachable(&mut self) {
        if self.probing {
            self.unreachable = true;
        }
    }
    pub fn finished(&mut self, failed: bool) {
        self.probing = false;
        if failed {
            self.pending = None;
        }
        if std::mem::take(&mut self.unreachable) {
            self.unreachable_streak = self.unreachable_streak.saturating_add(1);
            self.next = self
                .probed_at
                .saturating_add(unreachable_delay(self.unreachable_streak));
        } else {
            self.unreachable_streak = 0;
        }
    }
    pub fn reset(&mut self) {
        self.next = Duration::ZERO;
        self.pending = None;
        self.unreachable_streak = 0;
    }
}

/// Probes in a row that may fail before the minute cadence slows down.
const UNREACHABLE_BEFORE_BACKOFF: u32 = 2;
/// Longest wait between probes that cannot reach the feed.
const MAX_UNREACHABLE_DELAY: Duration = Duration::from_hours(1);

/// Wait from the start of a probe to the next after `streak` probes in a
/// row could not reach the feed: one minute at first, then doubling.
pub(super) fn unreachable_delay(streak: u32) -> Duration {
    let over = streak.saturating_sub(UNREACHABLE_BEFORE_BACKOFF).min(6);
    Duration::from_mins(1)
        .saturating_mul(1u32 << over)
        .min(MAX_UNREACHABLE_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn checks_immediately_then_each_minute_without_overlap_or_wake_bursts() {
        let mut s = UpdateSchedule::default();
        assert_eq!(s.next_action(at(0), false, true), Action::None);
        assert_eq!(s.next_action(at(0), true, false), Action::None);
        assert_eq!(s.next_action(at(0), true, true), Action::Probe);
        assert_eq!(s.next_action(at(60), true, true), Action::None);
        s.finished(false);
        assert_eq!(s.next_action(at(59), true, true), Action::None);
        assert_eq!(s.next_action(at(60), true, true), Action::Probe);
        s.finished(false);
        assert_eq!(s.next_action(at(600), true, true), Action::Probe);
        s.finished(false);
        assert_eq!(s.next_action(at(600), true, true), Action::None);
    }

    #[test]
    fn unreachable_feed_backs_off_only_after_repeated_failures() {
        let mut s = UpdateSchedule::default();
        let mut now = 0;
        // Two failures keep the minute cadence; then the wait doubles.
        for wait in [60, 60, 120, 240] {
            assert_eq!(s.next_action(at(now), true, true), Action::Probe);
            s.unreachable();
            s.finished(true);
            assert_eq!(s.next_action(at(now + wait - 1), true, true), Action::None);
            now += wait;
        }
        assert_eq!(s.next_action(at(now), true, true), Action::Probe);
        // "No update" is not unreachable: back to every minute.
        s.finished(true);
        assert_eq!(s.next_action(at(now + 60), true, true), Action::Probe);
        assert_eq!(unreachable_delay(100), Duration::from_hours(1));
    }

    #[test]
    fn not_ready_retries_without_consuming_the_minute() {
        let mut s = UpdateSchedule::default();
        assert_eq!(s.next_action(at(0), true, false), Action::None);
        assert_eq!(s.next_action(at(0), true, true), Action::Probe);
    }

    #[test]
    fn found_without_a_probe_does_not_offer() {
        let mut s = UpdateSchedule::default();
        s.found("1".into());
        s.finished(false);
        assert_eq!(s.next_action(at(0), true, true), Action::Probe);
    }

    #[test]
    fn only_successful_completed_probes_offer_each_version_once() {
        let mut s = UpdateSchedule::default();
        assert_eq!(s.next_action(at(0), true, true), Action::Probe);
        s.found("1".into());
        assert_eq!(s.next_action(at(1), true, true), Action::None);
        s.finished(true);
        assert_eq!(s.next_action(at(2), true, true), Action::None);
        assert_eq!(s.next_action(at(60), true, true), Action::Probe);
        s.found("1".into());
        s.finished(false);
        assert_eq!(s.next_action(at(61), true, false), Action::None);
        assert_eq!(s.next_action(at(61), true, true), Action::Offer);
        assert_eq!(s.next_action(at(120), true, true), Action::Probe);
        s.found("1".into());
        s.finished(false);
        assert_eq!(s.next_action(at(121), true, true), Action::None);
        s.reset();
        assert_eq!(s.next_action(at(122), true, true), Action::Probe);
        s.found("2".into());
        s.finished(false);
        assert_eq!(s.next_action(at(123), false, true), Action::None);
        assert_eq!(s.next_action(at(124), true, true), Action::None);
    }
}
