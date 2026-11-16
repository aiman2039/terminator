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

/// The generation whose dismiss poll has paused, and when. A successful
/// remove or [`DISMISS_ATTEMPTS`] misses both latch it until
/// [`ALERT_WAIT`] later. A different alert starts over.
#[cfg(target_os = "macos")]
static DISMISSED: Mutex<Option<(u64, std::time::Instant)>> = Mutex::new(None);

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
/// `ObjC` dismiss poll treats as an auto-dismiss and releases the waiter.
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

/// The waiter the worker is blocked on: its generation, when it started (so
/// [`pump`] can end it at [`ALERT_WAIT`] even if nothing newer is queued),
/// and its title (checked when part of the marker was stripped). The
/// generation is stored with the time so a timeout is applied only to the
/// alert it was measured for.
#[cfg(target_os = "macos")]
static PRESENTED: Mutex<Option<Presented>> = Mutex::new(None);

#[cfg(target_os = "macos")]
struct Presented {
    generation: u64,
    at: std::time::Instant,
    summary: String,
}

/// Monotonic submission order. Every [`send`] claims the next value.
static SUBMITTED: AtomicU64 = AtomicU64::new(0);

/// Generation of the alert currently waiting, or 0 when none is.
static PRESENTING: AtomicU64 = AtomicU64::new(0);

static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

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
    let Some(queue) = dispatcher() else {
        return;
    };
    let _ = queue.send(Queued {
        generation: next_generation(),
        alert,
    });
    wake();
}

fn next_generation() -> u64 {
    SUBMITTED.fetch_add(1, Ordering::Relaxed).saturating_add(1)
}

/// The notification queue, or `None` once the worker has failed to start.
/// A spawn failure is logged and notifications stay off so the daemon and
/// its sessions keep running.
fn dispatcher() -> Option<&'static Sender<Queued>> {
    static QUEUE: OnceLock<Option<Sender<Queued>>> = OnceLock::new();
    QUEUE
        .get_or_init(|| queue_or_disable(spawn_worker(present)))
        .as_ref()
}

/// Keeps the queue only when the worker thread exists. Spawn failure disables
/// notifications instead of exiting the process.
fn queue_or_disable(spawned: std::io::Result<Sender<Queued>>) -> Option<Sender<Queued>> {
    match spawned {
        Ok(sender) => Some(sender),
        Err(error) => {
            crate::console::Console::err(format_args!(
                "desktop notification worker failed to start: {error}"
            ));
            None
        }
    }
}

fn spawn_worker(present: impl Fn(Queued) + Send + 'static) -> std::io::Result<Sender<Queued>> {
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
        })?;
    Ok(tx)
}

fn present(Queued { generation, alert }: Queued) {
    ACTIVE.fetch_add(1, Ordering::Relaxed);
    PRESENTING.store(generation, Ordering::Relaxed);
    // Dropped after the wait so a panic still clears the deadline. The guard
    // outlives `ACTIVE` going back to zero, which stops [`pump`] first.
    let _waiter = Waiter::start(generation, &alert.summary);
    // Wake the daemon's event loop so it starts pumping the Cocoa run loop
    // that delivers this notification's action.
    wake();
    // On macOS the wait can be given up when the banner cannot be found, so
    // a stripped banner cannot keep the worker and the dismiss poll busy.
    #[cfg(target_os = "macos")]
    wait_or_abandon(generation, move || show_and_wait(alert, generation));
    #[cfg(not(target_os = "macos"))]
    show_and_wait(alert, generation);
    PRESENTING.store(0, Ordering::Relaxed);
    ACTIVE.fetch_sub(1, Ordering::Relaxed);
    // Wake again so the loop can stop pumping and block for connections.
    wake();
}

/// Arms the macOS wait deadline for the alert currently blocking the worker.
struct Waiter;

impl Waiter {
    fn start(generation: u64, summary: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            *PRESENTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Presented {
                generation,
                at: std::time::Instant::now(),
                summary: summary.to_owned(),
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (generation, summary);
        Self
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            *PRESENTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
    }
}

/// Start time and title of `generation`'s waiter, or `None` when the stored
/// waiter is another generation (one that ended, or one that just started).
#[cfg(target_os = "macos")]
fn presented_for(generation: u64) -> Option<(std::time::Instant, String)> {
    let presented = PRESENTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let at = started_for(presented.as_ref().map(|p| (p.generation, p.at)), generation)?;
    Some((at, presented.as_ref()?.summary.clone()))
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
    // GNOME keeps a plain notification in its list without closing it; a
    // transient one is closed when its banner goes.
    #[cfg(all(unix, not(target_os = "macos")))]
    notification.hint(notify_rust::Hint::Transient(true));
    let Ok(handle) = notification.show() else {
        return;
    };
    let on_action = |action: &str| {
        if action == "__closed" {
            return;
        }
        let _ = atomic_write(&paths.runtime.join("activation"), notice.as_bytes());
        launch_gui(&paths);
    };
    // The server may ignore the timeout: end the wait at ALERT_WAIT or when
    // a newer alert is queued, and close only this notification.
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(action) = linux_wait::wait(&handle, |started| {
        superseded() || started.elapsed() >= ALERT_WAIT
    }) {
        on_action(&action);
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    handle.wait_for_action(on_action);
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux_wait;

/// Whether a submitted alert is waiting behind the one currently presented.
/// The main loop preempts the active notification in that case so the newest
/// alert is shown immediately instead of after the previous waiter ends.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn superseded() -> bool {
    superseded_for(PRESENTING.load(Ordering::Relaxed))
}

/// Whether an alert newer than `generation` was submitted.
#[cfg(any(unix, test))]
fn superseded_for(generation: u64) -> bool {
    generation != 0 && SUBMITTED.load(Ordering::Relaxed) > generation
}

mod marker;
use marker::*;

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
    let _ = spawn_detached(command);
}

/// Starts `command` and reaps it on its own thread. On Linux the child is
/// the GUI itself: waiting on the notification worker would hold every
/// later notification until that GUI exits.
fn spawn_detached(mut command: Command) -> std::io::Result<()> {
    let mut child = command.spawn()?;
    std::thread::Builder::new()
        .name("gui-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
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

#[cfg(any(target_os = "macos", test))]
mod macos_helpers;
#[cfg(any(target_os = "macos", test))]
use macos_helpers::*;
#[cfg(any(target_os = "macos", test))]
mod abandon;
#[cfg(any(target_os = "macos", test))]
use abandon::*;
pub fn pump() {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::{base::TCFType, runloop::CFRunLoop, string::CFString};
        if ACTIVE.load(Ordering::Relaxed) > 0 {
            // Generation first: the start time and title must belong to it,
            // so a waiter that ends between the reads cannot time out the
            // next one.
            let generation = PRESENTING.load(Ordering::Relaxed);
            let presented = presented_for(generation);
            let now = std::time::Instant::now();
            let presented_at = presented.as_ref().map(|(at, _)| *at);
            if waiter_should_end(superseded_for(generation), presented_at, now) {
                let summary = presented.map(|(_, summary)| summary).unwrap_or_default();
                dismiss(generation, &summary, presented_at, now);
            }
            let mode = CFString::new("kCFRunLoopDefaultMode");
            let _ = CFRunLoop::run_in_mode(mode.as_concrete_TypeRef(), PUMP_INTERVAL, true);
        }
    }
}

/// One throttled try to remove `generation`'s banner, ending its waiter.
#[cfg(target_os = "macos")]
fn dismiss(
    generation: u64,
    summary: &str,
    presented_at: Option<std::time::Instant>,
    now: std::time::Instant,
) {
    let mut latch = DISMISSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if dismiss_paused(*latch, generation, now) || !dismiss_poll_due() {
        return;
    }
    // `deliveredNotifications` autoreleases the decoded list. Drain it here
    // so a miss does not retain that list for the life of the process.
    let removed = objc2::rc::autoreleasepool(|_| remove_active_notification(generation, summary));
    if !record_dismiss_attempt(generation, removed) {
        return;
    }
    // Not found after the misses and past the wait: stop waiting and leave
    // the stray banner. Else pause and try once more at the wait.
    if !removed && give_up_after_misses(presented_at, now) {
        abandon(generation);
    }
    *latch = Some((generation, now));
}

/// Remove this daemon's waiting banner before the process exits, so a
/// shutdown during the wait does not leave it in Notification Center. One
/// `deliveredNotifications` call; it does not wait for the waiter to end.
pub fn clear_on_exit() {
    #[cfg(target_os = "macos")]
    if ACTIVE.load(Ordering::Relaxed) > 0 {
        let generation = PRESENTING.load(Ordering::Relaxed);
        let summary = presented_for(generation)
            .map(|(_, summary)| summary)
            .unwrap_or_default();
        let _ = objc2::rc::autoreleasepool(|_| remove_active_notification(generation, &summary));
    }
}

#[cfg(test)]
mod tests;
