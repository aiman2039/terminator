use super::*;
#[cfg(target_os = "macos")]
pub(super) fn record_dismiss_attempt(generation: u64, removed: bool) -> bool {
    if removed {
        DISMISS_MISSES.store(0, Ordering::Relaxed);
        return true;
    }
    let misses = if DISMISS_GENERATION.swap(generation, Ordering::Relaxed) != generation {
        DISMISS_MISSES.store(1, Ordering::Relaxed);
        1
    } else {
        DISMISS_MISSES
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    };
    misses >= DISMISS_ATTEMPTS
}

/// Whether the throttled dismiss poll may run now, updating its timestamp.
#[cfg(target_os = "macos")]
pub(super) fn dismiss_poll_due() -> bool {
    let now = std::time::Instant::now();
    let mut last = LAST_DISMISS_POLL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match *last {
        Some(previous) if now.duration_since(previous) < DISMISS_POLL_INTERVAL => false,
        _ => {
            *last = Some(now);
            true
        }
    }
}

/// Whether the dismiss poll for `generation` is paused at `now`. A latch
/// (a remove, or [`DISMISS_ATTEMPTS`] misses) pauses it for [`ALERT_WAIT`],
/// not for good: a banner delivered after the misses, or one a title
/// fallback did not reach, is tried again, so the waiter cannot wait forever.
#[cfg(any(target_os = "macos", test))]
pub(super) fn dismiss_paused(
    latch: Option<(u64, std::time::Instant)>,
    generation: u64,
    now: std::time::Instant,
) -> bool {
    latch.is_some_and(|(latched, at)| {
        latched == generation && now.saturating_duration_since(at) < ALERT_WAIT
    })
}

/// `true` when the main thread should remove the delivered banner. A newer
/// alert does so immediately. An unanswered alert does so at [`ALERT_WAIT`],
/// which is the only way to stop `mac-notification-sys` polling Notification
/// Center for the rest of the process lifetime.
#[cfg(any(target_os = "macos", test))]
pub(super) fn waiter_should_end(
    superseded: bool,
    presented_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    superseded
        || presented_at.is_some_and(|started| now.saturating_duration_since(started) >= ALERT_WAIT)
}

/// The start time stored for `generation`'s waiter. A time stored for any
/// other generation is not used, so a waiter that ended (or one that just
/// started) between two reads can never time out the wrong alert.
#[cfg(any(target_os = "macos", test))]
pub(super) fn started_for(
    presented: Option<(u64, std::time::Instant)>,
    generation: u64,
) -> Option<std::time::Instant> {
    presented
        .filter(|&(stored, _)| generation != 0 && stored == generation)
        .map(|(_, at)| at)
}

/// Fewest marker characters a banner must keep to be taken as ours when
/// Notification Center stripped some. Live banners kept 59 to 63 of 65. At 57
/// another random 64-bit marker matches by chance at most about once in 2^31.
pub(super) const MIN_MARKER_CHARS: usize = 57;

/// Which delivered banners are this waiter's. A banner is ours when it
/// carries the whole marker, or when it has this waiter's title and the
/// lead sentence followed by at least [`MIN_MARKER_CHARS`] characters that
/// are, in order, part of this marker. A banner with no marker left, or with
/// another daemon's or generation's marker, is never ours.
#[cfg(any(target_os = "macos", test))]
pub(super) fn banners_to_remove(banners: &[Banner<'_>], marker: &str, summary: &str) -> Vec<usize> {
    banners
        .iter()
        .enumerate()
        .filter(|(_, banner)| {
            banner.text.contains(marker)
                || (!summary.is_empty()
                    && banner.title == summary
                    && banner
                        .text
                        .strip_prefix(BODY_LEAD)
                        .is_some_and(|rest| partial_marker(rest, marker)))
        })
        .map(|(index, _)| index)
        .collect()
}

/// `rest` is what survived of `marker`: only marker characters, enough of
/// them, and each in the order the marker has them.
fn partial_marker(rest: &str, marker: &str) -> bool {
    if rest.chars().count() < MIN_MARKER_CHARS {
        return false;
    }
    let mut remaining = marker.chars();
    rest.chars().all(|c| remaining.any(|m| m == c))
}

#[cfg(any(target_os = "macos", test))]
pub(super) struct Banner<'a> {
    pub(super) title: &'a str,
    pub(super) text: &'a str,
}
