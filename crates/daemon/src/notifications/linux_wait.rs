//! Linux/BSD: notify-rust's `wait_for_action` has no client timeout, and
//! some servers (GNOME Shell) ignore `expire_timeout`. Race the wait with a
//! deadline that fires at [`super::ALERT_WAIT`] or when a newer alert is
//! queued, then close only this notification so the worker moves on.
use std::time::{Duration, Instant};

/// How often the deadline asks `should_end`.
const POLL: Duration = Duration::from_millis(100);

/// Waits for an action on `handle`. Returns the action name, `__closed`
/// when the server closed it, or `None` when this side closed it because
/// `should_end` became true first. zbus drives the D-Bus socket on its own
/// async-io thread, so a current-thread tokio runtime only runs the race.
pub(super) fn wait(
    handle: &notify_rust::NotificationHandle,
    should_end: impl Fn(Instant) -> bool,
) -> Option<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let started = Instant::now();
    let mut action = None;
    let acted = runtime.block_on(async {
        let deadline = async {
            let mut tick = tokio::time::interval(POLL);
            while !should_end(started) {
                tick.tick().await;
            }
        };
        tokio::select! {
            () = handle.wait_for_action_async(|response| action = Some(action_name(response))) => true,
            () = deadline => false,
        }
    });
    if !acted {
        runtime.block_on(handle.close_async());
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
