//! Ending a waiter whose banner cannot be found.
//!
//! `mac-notification-sys` releases `wait_for_action` only on a click or when
//! the banner leaves Notification Center, and it has no cancel. When
//! Notification Center strips too much of the marker, the banner cannot be
//! proven ours and is not removed. The worker then stops waiting for that
//! alert: the blocked call stays on its own thread (its 0.5 s check runs only
//! while the main thread pumps for a later alert) and the stray banner stays.

use super::*;
use std::sync::Mutex;

/// The generation being waited for and the sender that ends its wait early.
static ABANDON: Mutex<Option<(u64, Sender<()>)>> = Mutex::new(None);

/// Runs `wait` for `generation` on its own thread and returns when it ends or
/// when [`abandon`] is called for this generation. When the thread cannot
/// start, the alert is skipped and logged.
pub(super) fn wait_or_abandon(generation: u64, wait: impl FnOnce() + Send + 'static) {
    let (tx, rx) = mpsc::channel::<()>();
    *slot() = Some((generation, tx.clone()));
    let spawned = std::thread::Builder::new()
        .name("desktop-notification-wait".into())
        .spawn(move || {
            // Signals on drop, so a panicking wait still releases the worker.
            let _done = Done(tx);
            wait();
        });
    match spawned {
        Ok(_) => {
            let _ = rx.recv();
        }
        Err(error) => crate::console::Console::err(format_args!(
            "desktop notification wait failed to start: {error}"
        )),
    }
    let mut slot = slot();
    if slot
        .as_ref()
        .is_some_and(|(stored, _)| *stored == generation)
    {
        *slot = None;
    }
}

struct Done(Sender<()>);

impl Drop for Done {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// Stop waiting for `generation`. Another generation's wait is not touched.
pub(super) fn abandon(generation: u64) -> bool {
    let slot = slot();
    match slot.as_ref() {
        Some((stored, tx)) if *stored == generation => tx.send(()).is_ok(),
        _ => false,
    }
}

fn slot() -> std::sync::MutexGuard<'static, Option<(u64, Sender<()>)>> {
    ABANDON
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether a waiter whose banner was not found in `DISMISS_ATTEMPTS` polls
/// is given up. Only once its [`ALERT_WAIT`] has passed: a newer alert that
/// cannot find a banner yet pauses and tries once more at the timeout.
pub(super) fn give_up_after_misses(
    presented_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    waiter_should_end(false, presented_at, now)
}
