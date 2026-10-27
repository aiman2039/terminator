use super::*;
use super::*;
use std::{
    collections::HashSet,
    sync::{Arc, Condvar, Mutex as StdMutex},
    time::{Duration, Instant},
};

fn temp_alert() -> DesktopAlert {
    DesktopAlert {
        paths: Paths::at(std::env::temp_dir()),
        summary: "Agent: Waiting".into(),
        notice: "notice".into(),
        sound: false,
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn dismiss_poll_is_throttled_between_pumps() {
    *LAST_DISMISS_POLL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    assert!(dismiss_poll_due(), "the first dismiss poll runs");
    assert!(
        !dismiss_poll_due(),
        "an immediate second poll is throttled to the interval"
    );

    DISMISS_MISSES.store(0, Ordering::Relaxed);
    DISMISS_GENERATION.store(0, Ordering::Relaxed);
    assert!(!record_dismiss_attempt(1, false), "one miss keeps polling");
    assert!(!record_dismiss_attempt(1, false));
    assert!(!record_dismiss_attempt(1, false));
    assert!(record_dismiss_attempt(1, false), "the fourth miss stops");
    assert!(
        !record_dismiss_attempt(2, false),
        "a new generation starts again at one miss"
    );
    assert!(record_dismiss_attempt(3, true), "a hit stops");
}

#[test]
fn os_banner_sound_name_is_platform_default() {
    let name = super::sound_name();
    assert!(!name.is_empty());
    if cfg!(target_os = "macos") {
        assert_eq!(name, "Glass");
    } else {
        assert_eq!(name, "message-new-instant");
    }
}

#[test]
fn unanswered_alert_ends_the_waiter_after_ten_seconds() {
    let start = Instant::now();
    assert!(
        !waiter_should_end(false, Some(start), start),
        "a fresh alert stays until the wait elapses"
    );
    assert!(
        !waiter_should_end(false, Some(start), start + Duration::from_secs(9)),
        "a click is still delivered during the wait"
    );
    assert!(
        waiter_should_end(false, Some(start), start + ALERT_WAIT),
        "an unanswered alert is removed at the wait"
    );
    assert!(
        waiter_should_end(true, Some(start), start),
        "a newer alert still replaces the waiting one immediately"
    );
    assert!(
        !waiter_should_end(false, None, start + ALERT_WAIT),
        "no armed waiter is not timed out"
    );
}

#[test]
fn release_keeps_other_banners_when_the_marker_is_intact() {
    let marker = "MARKER-1";
    let banners = [
        Banner {
            title: "Agent: Waiting",
            text: "Open the notification to view the session context.MARKER-1",
        },
        Banner {
            title: "Agent: Waiting",
            text: "Open the notification to view the session context.",
        },
    ];
    assert_eq!(
        banners_to_remove(&banners, marker, "Agent: Waiting"),
        vec![0],
        "a marker hit does not clear another generation's same title"
    );
}

#[test]
fn release_falls_back_to_the_title_when_the_marker_was_stripped() {
    let banners = [
        Banner {
            title: "Agent: Waiting",
            text: "Open the notification to view the session context.",
        },
        Banner {
            title: "Agent: Done",
            text: "Open the notification to view the session context.",
        },
    ];
    assert_eq!(
        banners_to_remove(&banners, "MARKER-1", "Agent: Waiting"),
        vec![0]
    );
    assert!(
        banners_to_remove(&banners, "MARKER-1", "").is_empty(),
        "an empty title does not match every banner"
    );
}

#[test]
fn marker_is_hidden_and_unique_per_generation() {
    let first = marker_for(1);
    let second = marker_for(2);
    assert_ne!(first, second);
    assert_eq!(first.chars().count(), 65);
    assert!(
        first
            .chars()
            .all(|c| matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}'))
    );
}

/// A presenter that blocks each generation until the test releases it, so
/// the test can observe how many waiters are live and in what order.
struct Gate {
    released: StdMutex<HashSet<u64>>,
    entered: StdMutex<Vec<u64>>,
    presented: StdMutex<Vec<u64>>,
    live: AtomicUsize,
    peak: AtomicUsize,
    cv: Condvar,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            released: StdMutex::new(HashSet::new()),
            entered: StdMutex::new(Vec::new()),
            presented: StdMutex::new(Vec::new()),
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            cv: Condvar::new(),
        })
    }

    fn present(&self, generation: u64) {
        let live = self.live.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        self.peak.fetch_max(live, Ordering::SeqCst);
        self.entered.lock().unwrap().push(generation);
        self.cv.notify_all();
        let mut released = self.released.lock().unwrap();
        while !released.contains(&generation) {
            released = self.cv.wait(released).unwrap();
        }
        drop(released);
        self.presented.lock().unwrap().push(generation);
        self.live.fetch_sub(1, Ordering::SeqCst);
    }

    fn release(&self, generation: u64) {
        self.released.lock().unwrap().insert(generation);
        self.cv.notify_all();
    }

    fn entered(&self, generation: u64) -> bool {
        self.entered.lock().unwrap().contains(&generation)
    }

    fn presented(&self) -> Vec<u64> {
        self.presented.lock().unwrap().clone()
    }
}

fn gate_alert(generation: u64) -> Queued {
    Queued {
        generation,
        alert: temp_alert(),
    }
}

#[test]
fn a_failed_notification_worker_disables_alerts_without_exiting() {
    let queue = queue_or_disable(Err(std::io::Error::other("resource unavailable")));
    assert!(queue.is_none(), "notifications stay disabled");
}

#[test]
fn notification_waiters_are_bounded_and_coalesce() {
    const N: u64 = 32;
    let gate = Gate::new();
    let presented = Arc::clone(&gate);
    let worker = spawn_worker(move |queued| presented.present(queued.generation))
        .expect("spawn desktop notification worker");

    worker.send(gate_alert(1)).expect("queue first alert");
    wait_until(|| gate.entered(1), "no waiter started");
    assert_eq!(gate.live.load(Ordering::SeqCst), 1, "one waiter is active");
    assert_eq!(
        gate.peak.load(Ordering::SeqCst),
        1,
        "only one waiter at a time"
    );

    for generation in 2..=N {
        worker.send(gate_alert(generation)).expect("queue alert");
    }
    assert_eq!(
        gate.presented(),
        Vec::<u64>::new(),
        "the active waiter has not finished"
    );

    gate.release(1);
    wait_until(|| gate.entered(N), "coalesced alert was not presented");
    assert_eq!(gate.presented(), vec![1], "only the first finished");
    gate.release(N);
    wait_until(
        || gate.presented().len() == 2,
        "coalesced alert never finished",
    );
    assert_eq!(
        gate.presented(),
        vec![1, N],
        "a burst coalesces to the newest alert"
    );
    assert_eq!(
        gate.peak.load(Ordering::SeqCst),
        1,
        "only one waiter at a time"
    );
}

#[test]
fn newer_alert_supersedes_the_waiting_one() {
    SUBMITTED.store(0, Ordering::SeqCst);
    PRESENTING.store(0, Ordering::SeqCst);
    ACTIVE.store(0, Ordering::SeqCst);
    let gate = Gate::new();
    let presented = Arc::clone(&gate);
    let worker = spawn_worker(move |queued| {
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        PRESENTING.store(queued.generation, Ordering::Relaxed);
        presented.present(queued.generation);
        PRESENTING.store(0, Ordering::Relaxed);
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    })
    .expect("spawn desktop notification worker");

    let first = next_generation();
    worker.send(gate_alert(first)).expect("queue first alert");
    wait_until(|| gate.entered(first), "first waiter never started");
    assert!(!superseded(), "nothing has superseded the first waiter");

    let second = next_generation();
    worker.send(gate_alert(second)).expect("queue second alert");
    assert!(
        superseded(),
        "the queued alert must supersede the waiting one"
    );

    // The daemon main loop removes the waiting notification, which releases
    // the presenter and lets the newer alert take its place.
    gate.release(first);
    wait_until(|| gate.entered(second), "the newer alert was not presented");
    assert_eq!(gate.presented(), vec![first]);
    assert_eq!(
        gate.peak.load(Ordering::SeqCst),
        1,
        "only one waiter at a time"
    );

    gate.release(second);
    wait_until(
        || gate.presented().len() == 2,
        "second waiter never finished",
    );
    assert_eq!(gate.presented(), vec![first, second]);
    assert_eq!(
        gate.peak.load(Ordering::SeqCst),
        1,
        "only one waiter at a time"
    );
}
