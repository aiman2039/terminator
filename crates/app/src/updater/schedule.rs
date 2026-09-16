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
        self.next = now + Duration::from_secs(60);
        self.probing = true;
        Action::Probe
    }
    pub fn found(&mut self, version: String) {
        if self.probing {
            self.pending = Some(version);
        }
    }
    pub fn finished(&mut self, failed: bool) {
        self.probing = false;
        if failed {
            self.pending = None;
        }
    }
    pub fn reset(&mut self) {
        self.next = Duration::ZERO;
        self.pending = None;
    }
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
