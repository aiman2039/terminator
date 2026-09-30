#[cfg(target_os = "macos")]
use std::sync::Mutex;
use std::{
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Sender},
    },
};
use terminator_core::{Paths, atomic_write};

/// How long the daemon main thread services the Cocoa run loop per pump while
/// a notification awaits an action. Kept short so a pending notification does
/// not burn measurable CPU; the dismiss poll only needs to run every 0.5 s.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// How often the expensive dismiss poll may run. The run loop is serviced far
/// more often so a click is handled promptly, but finding the waiting alert
/// calls `deliveredNotifications`, a synchronous XPC into `usernoted`, so it is
/// throttled instead of repeating on every pump. Without this, a notification
/// that stays unanswered while a newer one queues behind it makes the main
/// loop enumerate Notification Center ~10x/s forever.
#[cfg(target_os = "macos")]
const DISMISS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// When the dismiss poll last ran, so [`pump`] can throttle it.
#[cfg(target_os = "macos")]
static LAST_DISMISS_POLL: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// The generation whose dismiss poll has stopped. A successful remove or
/// [`DISMISS_ATTEMPTS`] misses both latch it. A different alert starts over.
#[cfg(target_os = "macos")]
static DISMISSED: AtomicU64 = AtomicU64::new(0);

/// How many times one generation may miss its delivered alert before the
/// dismiss poll stops. One miss keeps polling so a banner that is not
/// registered yet can still be preempted. Four misses is two seconds.
#[cfg(target_os = "macos")]
const DISMISS_ATTEMPTS: u64 = 4;

/// Misses recorded for [`DISMISS_GENERATION`].
#[cfg(target_os = "macos")]
static DISMISS_MISSES: AtomicU64 = AtomicU64::new(0);

/// Generation the miss count belongs to. A different generation starts at one.
#[cfg(target_os = "macos")]
static DISMISS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Notifications that are shown but not yet acted on. A single long-lived
/// worker owns every wait, so at most one waiter exists at a time. A newer
/// alert preempts it by removing only its delivered notification, which the
/// ObjC dismiss poll treats as an auto-dismiss and releases the waiter.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// How long one alert may keep the worker blocked. macOS ignores
/// [`notify_rust::Notification::timeout`], and `wait_for_action` polls
/// Notification Center every 0.5 s until the banner disappears. Capping the
/// wait is what stops that poll.
const ALERT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Stable lead-in of every alert body. The per-waiter marker is appended, but
/// Notification Center may strip those zero-width characters, so a timed-out
/// remove can fall back to this sentence plus the alert title.
const BODY_LEAD: &str = "Open the notification to view the session context.";

/// When the current waiter started blocking, so [`pump`] can end it at
/// [`ALERT_WAIT`] even if nothing newer is queued.
#[cfg(target_os = "macos")]
static PRESENTED_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Title of the alert the worker is waiting on. Used only when the invisible
/// marker is no longer in the delivered text.
#[cfg(target_os = "macos")]
static PRESENTING_SUMMARY: Mutex<String> = Mutex::new(String::new());

/// Monotonic submission order. Every [`send`] claims the next value.
static SUBMITTED: AtomicU64 = AtomicU64::new(0);

/// Generation of the alert currently waiting, or 0 when none is.
static PRESENTING: AtomicU64 = AtomicU64::new(0);

static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static NONCE: OnceLock<u64> = OnceLock::new();

pub struct DesktopAlert {
    pub paths: Paths,
    pub summary: String,
    pub notice: String,
    pub sound: bool,
}

/// One queued alert tagged with its submission order so the main loop can tell
/// whether a newer alert has arrived behind the one that is waiting.
struct Queued {
    generation: u64,
    alert: DesktopAlert,
}

pub fn initialize() {
    #[cfg(target_os = "macos")]
    {
        let _ = notify_rust::set_application("dev.terminator.app");
    }
}

fn sound_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Glass"
    } else {
        "message-new-instant"
    }
}

/// Queue a desktop alert without blocking the caller. One long-lived worker
/// owns every wait, so the number of live waiters is bounded to one. Waking
/// the daemon lets the main loop end a waiter whose notification a newer alert
/// has superseded.
pub fn send(alert: DesktopAlert) {
    let _ = dispatcher().send(Queued {
        generation: next_generation(),
        alert,
    });
    wake();
}

fn next_generation() -> u64 {
    SUBMITTED.fetch_add(1, Ordering::Relaxed) + 1
}

fn dispatcher() -> &'static Sender<Queued> {
    static QUEUE: OnceLock<Sender<Queued>> = OnceLock::new();
    QUEUE.get_or_init(|| spawn_worker(present))
}

fn spawn_worker(present: impl Fn(Queued) + Send + 'static) -> Sender<Queued> {
    let (tx, rx) = mpsc::channel::<Queued>();
    std::thread::Builder::new()
        .name("desktop-notifications".into())
        .spawn(move || {
            while let Ok(queued) = rx.recv() {
                // Coalesce a burst that piled up behind an active notification,
                // so acting on one alert does not replay a stale nudge per
                // deadline afterwards.
                let mut latest = queued;
                while let Ok(more_recent) = rx.try_recv() {
                    latest = more_recent;
                }
                present(latest);
            }
        })
        .expect("spawn desktop notification worker");
    tx
}

fn present(Queued { generation, alert }: Queued) {
    ACTIVE.fetch_add(1, Ordering::Relaxed);
    PRESENTING.store(generation, Ordering::Relaxed);
    // Dropped after the wait so a panic still clears the deadline. The guard
    // outlives `ACTIVE` going back to zero, which stops [`pump`] first.
    let _waiter = Waiter::start(&alert.summary);
    // Wake the daemon's event loop so it starts pumping the Cocoa run loop
    // that delivers this notification's action.
    wake();
    show_and_wait(alert, generation);
    PRESENTING.store(0, Ordering::Relaxed);
    ACTIVE.fetch_sub(1, Ordering::Relaxed);
    // Wake again so the loop can stop pumping and block for connections.
    wake();
}

/// Arms the macOS wait deadline for the alert currently blocking the worker.
struct Waiter;

impl Waiter {
    fn start(summary: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            *PRESENTED_AT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(std::time::Instant::now());
            *PRESENTING_SUMMARY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = summary.to_owned();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = summary;
        Self
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            *PRESENTED_AT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            PRESENTING_SUMMARY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
        }
    }
}

fn show_and_wait(
    DesktopAlert {
        paths,
        summary,
        notice,
        sound,
    }: DesktopAlert,
    generation: u64,
) {
    // The trailing marker is invisible but lets the main thread find exactly
    // this notification in Notification Center without touching another
    // generation's delivered alerts.
    let body = format!("{BODY_LEAD}{}", marker_for(generation));
    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&summary)
        .body(&body)
        .appname("Terminator")
        .action("default", "Open context")
        // Honored on Linux. macOS ignores it; [`pump`] removes the banner at
        // [`ALERT_WAIT`] so `wait_for_action` returns and the 0.5 s poll stops.
        .timeout(ALERT_WAIT);
    if sound {
        notification.sound_name(sound_name());
    }
    let Ok(handle) = notification.show() else {
        return;
    };
    handle.wait_for_action(|action| {
        if action == "__closed" {
            return;
        }
        let _ = atomic_write(&paths.runtime.join("activation"), notice.as_bytes());
        launch_gui(&paths);
    });
}

/// Whether a submitted alert is waiting behind the one currently presented.
/// The main loop preempts the active notification in that case so the newest
/// alert is shown immediately instead of after the previous waiter ends.
#[cfg(any(target_os = "macos", test))]
fn superseded() -> bool {
    let presenting = PRESENTING.load(Ordering::Relaxed);
    presenting != 0 && SUBMITTED.load(Ordering::Relaxed) > presenting
}

/// A unique, invisible suffix for one waiter. Zero-width joiners, spaces, and
/// non-joiners encode a per-process nonce mixed with the generation, so
/// another Terminator generation can never collide with this marker.
fn marker_for(generation: u64) -> String {
    let key = process_nonce() ^ generation;
    let mut marker = String::with_capacity(65);
    marker.push('\u{200D}');
    for bit in (0..64).rev() {
        marker.push(if (key >> bit) & 1 == 1 {
            '\u{200C}'
        } else {
            '\u{200B}'
        });
    }
    marker
}

fn process_nonce() -> u64 {
    *NONCE.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0);
        (u64::from(std::process::id())).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ now
    })
}

/// Remove only the waiting notification from Notification Center. Its
/// disappearance makes the ObjC dismiss poll resolve an auto-dismiss, releasing
/// the waiter so the worker can present the alert that superseded it. Returns
/// whether a matching delivered notification was found and removed.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn remove_active_notification() -> bool {
    let generation = PRESENTING.load(Ordering::Relaxed);
    if generation == 0 {
        return false;
    }
    let marker = marker_for(generation);
    let summary = PRESENTING_SUMMARY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    use objc2_foundation::NSUserNotificationCenter;
    let center = NSUserNotificationCenter::defaultUserNotificationCenter();
    // One fetch. Marker hits are preferred so a match cannot also clear
    // another generation's banner via the title fallback.
    let delivered = center.deliveredNotifications();
    let snapshots = delivered
        .iter()
        .map(|notification| {
            (
                notification
                    .title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
                notification
                    .informativeText()
                    .map(|text| text.to_string())
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let banners = snapshots
        .iter()
        .map(|(title, text)| Banner { title, text })
        .collect::<Vec<_>>();
    let indexes = banners_to_remove(&banners, &marker, &summary);
    if indexes.is_empty() {
        return false;
    }
    for (index, notification) in delivered.iter().enumerate() {
        if indexes.contains(&index) {
            center.removeDeliveredNotification(&notification);
        }
    }
    true
}

fn launch_gui(paths: &Paths) {
    let Ok(exe) = std::env::var_os("TERMINATOR_GUI_EXECUTABLE")
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
    else {
        return;
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("open");
        if let Some(bundle) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
        {
            c.arg("-a")
                .arg(bundle)
                .args(["--args", "--data-dir"])
                .arg(&paths.data);
        } else {
            c.arg("-a").arg(exe.with_file_name("terminator"));
        }
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = {
        let mut c = Command::new(exe.with_file_name("terminator"));
        c.arg("--data-dir").arg(&paths.data);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        let _ = child.wait();
    }
}

/// Install the daemon event-loop waker. It is called when a waiter starts or
/// ends so the main thread can block indefinitely while nothing is pending.
pub fn set_waker(waker: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(waker));
}

/// Wake the daemon event loop. Called when a waiter starts or ends, when a
/// newer alert must preempt the active one, and when the daemon must stop
/// accepting connections.
pub fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}

/// Whether the daemon main thread must pump the Cocoa run loop because a
/// notification is waiting for an action. False elsewhere: other platforms
/// deliver actions without a run-loop pump.
pub fn waiting() -> bool {
    cfg!(target_os = "macos") && ACTIVE.load(Ordering::Relaxed) > 0
}

/// `true` when this generation should stop polling. A remove stops at once.
/// Misses stop only at [`DISMISS_ATTEMPTS`]. A new generation starts at one.
#[cfg(target_os = "macos")]
fn record_dismiss_attempt(generation: u64, removed: bool) -> bool {
    if removed {
        DISMISS_MISSES.store(0, Ordering::Relaxed);
        return true;
    }
    let misses = if DISMISS_GENERATION.swap(generation, Ordering::Relaxed) != generation {
        DISMISS_MISSES.store(1, Ordering::Relaxed);
        1
    } else {
        DISMISS_MISSES.fetch_add(1, Ordering::Relaxed) + 1
    };
    misses >= DISMISS_ATTEMPTS
}

/// Whether the throttled dismiss poll may run now, updating its timestamp.
#[cfg(target_os = "macos")]
fn dismiss_poll_due() -> bool {
    let now = std::time::Instant::now();
    let mut last = LAST_DISMISS_POLL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match *last {
        Some(previous) if now.duration_since(previous) < DISMISS_POLL_INTERVAL => false,
        _ => {
            *last = Some(now);
            true
        }
    }
}

/// `true` when the main thread should remove the delivered banner. A newer
/// alert does so immediately. An unanswered alert does so at [`ALERT_WAIT`],
/// which is the only way to stop `mac-notification-sys` polling Notification
/// Center for the rest of the process lifetime.
#[cfg(any(target_os = "macos", test))]
fn waiter_should_end(
    superseded: bool,
    presented_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    superseded
        || presented_at.is_some_and(|started| now.saturating_duration_since(started) >= ALERT_WAIT)
}

/// Which delivered banners to remove. Marker hits win, so a banner that still
/// carries this waiter's marker is the only one cleared. The title fallback
/// runs only when Notification Center kept the sentence and dropped the marker.
#[cfg(any(target_os = "macos", test))]
fn banners_to_remove(banners: &[Banner<'_>], marker: &str, summary: &str) -> Vec<usize> {
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
        .filter(|(_, banner)| banner.title == summary && banner.text.starts_with(BODY_LEAD))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
struct Banner<'a> {
    title: &'a str,
    text: &'a str,
}

/// Service the Cocoa run loop once. Cocoa delivers notification callbacks on
/// the daemon main thread, so this must be called there while a waiter exists.
/// A newer alert, or one that has been waiting for [`ALERT_WAIT`], is removed
/// so the ObjC dismiss poll releases the worker.
pub fn pump() {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::{base::TCFType, runloop::CFRunLoop, string::CFString};
        if ACTIVE.load(Ordering::Relaxed) > 0 {
            let presented_at = *PRESENTED_AT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if waiter_should_end(superseded(), presented_at, std::time::Instant::now()) {
                let generation = PRESENTING.load(Ordering::Relaxed);
                if DISMISSED.load(Ordering::Relaxed) != generation && dismiss_poll_due() {
                    // `deliveredNotifications` autoreleases the decoded list.
                    // Drain it here so a miss does not retain that list for
                    // the life of the process.
                    let removed = objc2::rc::autoreleasepool(|_| remove_active_notification());
                    if record_dismiss_attempt(generation, removed) {
                        DISMISSED.store(generation, Ordering::Relaxed);
                    }
                }
            }
            let mode = CFString::new("kCFRunLoopDefaultMode");
            let _ = CFRunLoop::run_in_mode(mode.as_concrete_TypeRef(), PUMP_INTERVAL, true);
        }
    }
}

#[cfg(test)]
mod tests {
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
        let deadline = Instant::now() + Duration::from_secs(10);
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
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
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
            let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
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
    fn notification_waiters_are_bounded_and_coalesce() {
        const N: u64 = 32;
        let gate = Gate::new();
        let presented = gate.clone();
        let worker = spawn_worker(move |queued| presented.present(queued.generation));

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
        let presented = gate.clone();
        let worker = spawn_worker(move |queued| {
            ACTIVE.fetch_add(1, Ordering::Relaxed);
            PRESENTING.store(queued.generation, Ordering::Relaxed);
            presented.present(queued.generation);
            PRESENTING.store(0, Ordering::Relaxed);
            ACTIVE.fetch_sub(1, Ordering::Relaxed);
        });

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
}
