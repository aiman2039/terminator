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

/// Which delivered banners to remove. Marker hits win, so a banner that still
/// carries this waiter's marker is the only one cleared. The title fallback
/// runs only when Notification Center kept the sentence and dropped the marker:
/// a banner that still carries any marker belongs to another waiter (or
/// another daemon with the same bundle id) and is never cleared.
#[cfg(any(target_os = "macos", test))]
pub(super) fn banners_to_remove(banners: &[Banner<'_>], marker: &str, summary: &str) -> Vec<usize> {
    let marked = banners
        .iter()
        .enumerate()
        .filter(|(_, banner)| banner.text.contains(marker))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if !marked.is_empty() {
        return marked;
    }
    if summary.is_empty() {
        return Vec::new();
    }
    banners
        .iter()
        .enumerate()
        .filter(|(_, banner)| banner.title == summary && banner.text == BODY_LEAD)
        .map(|(index, _)| index)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
pub(super) struct Banner<'a> {
    pub(super) title: &'a str,
    pub(super) text: &'a str,
}
