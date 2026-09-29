use std::{
    process::{Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Sender},
    },
    time::{Duration, Instant},
};
use terminator_core::{Paths, atomic_write};

/// At most one desktop notification waits for a click at a time. On macOS every
/// waiter installs its own repeating 0.5 s Cocoa timer that queries
/// `deliveredNotifications` on the main run loop, so a backlog of waiters keeps
/// the notification service and the daemon's run loop busy for the life of the
/// process.
const MAX_WAITERS: usize = 1;

/// A pending notification is cleared from Notification Center after this long,
/// which ends its waiter instead of leaking it until the user acts.
const WAITER_DEADLINE: Duration = Duration::from_secs(60);

/// How long the daemon main thread services the Cocoa run loop per pump while
/// a notification awaits an action.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: Duration = Duration::from_millis(5);

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DEADLINE: Mutex<Option<Instant>> = Mutex::new(None);
static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

pub struct DesktopAlert {
    pub paths: Paths,
    pub summary: String,
    pub notice: String,
    pub sound: bool,
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
/// owns every wait, so the number of live waiters is bounded by construction
/// rather than by a counter that a stalled notification can leak past.
pub fn send(alert: DesktopAlert) {
    let _ = dispatcher().send(alert);
}

fn dispatcher() -> &'static Sender<DesktopAlert> {
    static QUEUE: OnceLock<Sender<DesktopAlert>> = OnceLock::new();
    QUEUE.get_or_init(|| spawn_worker(present))
}

fn spawn_worker(present: impl Fn(DesktopAlert) + Send + 'static) -> Sender<DesktopAlert> {
    let (tx, rx) = mpsc::channel::<DesktopAlert>();
    std::thread::Builder::new()
        .name("desktop-notifications".into())
        .spawn(move || {
            while let Ok(alert) = rx.recv() {
                // Coalesce a burst: keep only the newest alert waiting behind
                // an active notification, so a backlog cannot replay stale
                // nudges one per deadline after the user finally acts.
                let mut latest = alert;
                while let Ok(more_recent) = rx.try_recv() {
                    latest = more_recent;
                }
                present(latest);
            }
        })
        .expect("spawn desktop notification worker");
    tx
}

fn present(alert: DesktopAlert) {
    if ACTIVE.load(Ordering::Relaxed) >= MAX_WAITERS {
        return;
    }
    ACTIVE.fetch_add(1, Ordering::Relaxed);
    *DEADLINE.lock().unwrap() = Some(Instant::now() + WAITER_DEADLINE);
    // Wake the daemon's event loop so it starts pumping the Cocoa run loop
    // that delivers this notification's action.
    wake();
    show_and_wait(alert);
    *DEADLINE.lock().unwrap() = None;
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
) {
    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&summary)
        .body("Open the notification to view the session context.")
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

/// Wake the daemon event loop. Called when a waiter starts or ends, and when
/// the daemon must stop accepting connections.
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
pub fn pump() {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::{base::TCFType, runloop::CFRunLoop, string::CFString};
        if ACTIVE.load(Ordering::Relaxed) > 0 {
            expire_waiters();
            let mode = CFString::new("kCFRunLoopDefaultMode");
            let _ = CFRunLoop::run_in_mode(mode.as_concrete_TypeRef(), PUMP_INTERVAL, true);
        }
    }
}

/// End a waiter that has outlived [`WAITER_DEADLINE`] by clearing delivered
/// notifications. The ObjC side sees its notification disappear, treats it as
/// an auto-dismiss, and releases the waiter.
#[cfg(target_os = "macos")]
fn expire_waiters() {
    let now = Instant::now();
    let mut guard = DEADLINE.lock().unwrap();
    let Some(deadline) = *guard else {
        return;
    };
    if now < deadline {
        return;
    }
    // Clear first so the elapsed deadline is not observed again while the
    // notification center processes the removal.
    *guard = None;
    drop(guard);
    clear_delivered_notifications();
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn clear_delivered_notifications() {
    use objc2_foundation::NSUserNotificationCenter;
    let center = NSUserNotificationCenter::defaultUserNotificationCenter();
    center.removeAllDeliveredNotifications();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex as StdMutex};

    fn temp_alert() -> DesktopAlert {
        DesktopAlert {
            paths: Paths::at(std::env::temp_dir()),
            summary: "Agent: Waiting".into(),
            notice: "notice".into(),
            sound: false,
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
    fn notification_waiters_are_bounded() {
        const N: usize = 32;
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((StdMutex::new(false), Condvar::new()));
        let entered = Arc::new((StdMutex::new(false), Condvar::new()));
        let worker = spawn_worker({
            let active = active.clone();
            let peak = peak.clone();
            let gate = gate.clone();
            let entered = entered.clone();
            move |_alert| {
                let live = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(live, Ordering::SeqCst);
                let (lock, cv) = &*entered;
                *lock.lock().unwrap() = true;
                cv.notify_all();
                let (lock, cv) = &*gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = cv.wait(released).unwrap();
                }
                active.fetch_sub(1, Ordering::SeqCst);
            }
        });
        for _ in 0..N {
            worker.send(temp_alert()).expect("queue alert");
        }
        {
            let (lock, cv) = &*entered;
            let mut seen = lock.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !*seen {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "no waiter started");
                let (guard, _) = cv.wait_timeout(seen, remaining).unwrap();
                seen = guard;
            }
        }
        assert_eq!(
            active.load(Ordering::SeqCst),
            1,
            "the waiter is still active"
        );
        assert_eq!(peak.load(Ordering::SeqCst), 1, "only one waiter at a time");
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
}
