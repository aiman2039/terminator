use std::time::{Duration, Instant};

use super::super::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_stays_quiet_within_the_first_second() {
        let start = Instant::now();
        let mut probe = RepaintProbe {
            frames: 0,
            last: start,
        };
        assert!(probe.sample(start, &[]).is_none());
        assert_eq!(probe.frames, 1);
    }
    #[test]
    fn probe_reports_frames_and_causes_once_per_second() {
        let start = Instant::now();
        let mut probe = RepaintProbe {
            frames: 0,
            last: start,
        };
        let later = start + Duration::from_millis(1001);
        let line = probe
            .sample(later, &["crates/app/src/lib.rs:4978 1s".into()])
            .expect("a second has elapsed");
        assert!(line.contains("1 frames/s"), "{line}");
        assert!(line.contains("crates/app/src/lib.rs:4978"), "{line}");
        assert_eq!(probe.frames, 0);
        assert!(probe.sample(later, &[]).is_none());
    }
}
