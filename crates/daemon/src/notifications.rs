use std::{
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Sender},
    },
    time::Duration,
};
use terminator_core::{Paths, atomic_write};

/// How long the daemon main thread services the Cocoa run loop per pump while
/// a notification awaits an action. Kept short so a pending notification does
/// not burn measurable CPU; the dismiss poll only needs to run every 0.5 s.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: Duration = Duration::from_millis(1);

/// Notifications that are shown but not yet acted on. A single long-lived
/// worker owns every wait, so at most one waiter exists at a time. A newer
/// alert preempts it by removing only its delivered notification, which the
/// ObjC dismiss poll treats as an auto-dismiss and releases the waiter.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

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
    // Wake the daemon's event loop so it starts pumping the Cocoa run loop
    // that delivers this notification's action.
    wake();
    show_and_wait(alert, generation);
    PRESENTING.store(0, Ordering::Relaxed);
    ACTIVE.fetch_sub(1, Ordering::Relaxed);
    // Wake again so the loop can stop pumping and block for connections.
    wake();
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
    let body = format!(
        "Open the notification to view the session context.{}",
        marker_for(generation)
    );
    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&summary)
        .body(&body)
        .appname("Terminator")
        .action("default", "Open context")
        .timeout(10000);
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
/// the waiter so the worker can present the alert that superseded it.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn remove_active_notification() {
    let generation = PRESENTING.load(Ordering::Relaxed);
    if generation == 0 {
        return;
    }
    let marker = marker_for(generation);
    use objc2_foundation::NSUserNotificationCenter;
    let center = NSUserNotificationCenter::defaultUserNotificationCenter();
    for notification in center.deliveredNotifications().iter() {
        let matches = notification
            .informativeText()
            .is_some_and(|text| text.to_string().ends_with(marker.as_str()));
        if matches {
            center.removeDeliveredNotification(&notification);
        }
    }
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

/// Service the Cocoa run loop once. Cocoa delivers notification callbacks on
/// the daemon main thread, so this must be called there while a waiter exists.
/// A newer alert first removes the waiting notification so its waiter ends and
/// the worker can show the replacement.
pub fn pump() {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::{base::TCFType, runloop::CFRunLoop, string::CFString};
        if ACTIVE.load(Ordering::Relaxed) > 0 {
            if superseded() {
                remove_active_notification();
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
        time::Instant,
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
