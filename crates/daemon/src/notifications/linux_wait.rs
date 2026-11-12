//! Linux/BSD: notify-rust's `wait_for_action` has no client timeout, and
//! some servers (GNOME Shell) ignore `expire_timeout`. Race the wait with a
//! deadline that fires at [`super::ALERT_WAIT`] or when a newer alert is
//! queued, then close only this notification so the worker moves on.
use futures_util::{
    future::{Either, select},
    task::AtomicWaker,
};
use std::{
    future::Future,
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

/// Set once the wait must end; wakes the future that waits on it.
#[derive(Default)]
struct Deadline {
    fired: AtomicBool,
    finished: AtomicBool,
    waker: AtomicWaker,
}

impl Deadline {
    fn fire(&self) {
        self.fired.store(true, Ordering::Release);
        self.waker.wake();
    }
}

impl Future for &Deadline {
    type Output = ();
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.waker.register(cx.waker());
        if self.fired.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Waits for an action on `handle`. Returns the action name, `__closed`
/// when the server closed it, or `None` when this side closed it because
/// `should_end` became true first.
pub(super) fn wait(
    handle: &notify_rust::NotificationHandle,
    should_end: impl Fn(Instant) -> bool + Send + 'static,
) -> Option<String> {
    let deadline = Arc::new(Deadline::default());
    let watcher = Arc::clone(&deadline);
    let started = Instant::now();
    let _ = thread::Builder::new()
        .name("desktop-notification-deadline".into())
        .spawn(move || {
            while !watcher.finished.load(Ordering::Acquire) {
                if should_end(started) {
                    watcher.fire();
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
    let mut action = None;
    let acted = block_on(async {
        let wait = pin!(handle.wait_for_action_async(|response| {
            action = Some(action_name(response));
        }));
        matches!(select(wait, pin!(&*deadline)).await, Either::Left(_))
    });
    deadline.finished.store(true, Ordering::Release);
    if !acted {
        block_on(handle.close_async());
    }
    action.flatten()
}

fn action_name(response: &notify_rust::NotificationResponse) -> Option<String> {
    match response {
        notify_rust::NotificationResponse::Default => Some("default".into()),
        notify_rust::NotificationResponse::Action(action) => Some(action.clone()),
        notify_rust::NotificationResponse::Closed(_) => Some("__closed".into()),
        // XDG has no inline replies.
        notify_rust::NotificationResponse::Reply(_) => None,
    }
}

struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs a future on this thread. zbus's async-io reactor thread drives the
/// D-Bus socket, so parking until a wake is enough.
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        thread::park();
    }
}
