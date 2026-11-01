use std::{
    sync::{Weak, mpsc},
    thread,
};
use terminator_core::State;

use crate::notifications;
use crate::server::{Shared, relock};

/// Turns queued notification ids into desktop alerts. No throttle here: a
/// newer alert replaces the pending one immediately, and
/// `notifications::send` coalesces bursts.
pub(crate) struct AlertWorker;

impl AlertWorker {
    pub(crate) fn spawn(weak: Weak<Shared>, alerts: mpsc::Receiver<String>) {
        thread::spawn(move || {
            while let Ok(nid) = alerts.recv() {
                let Some(s) = weak.upgrade() else { break };
                Self::deliver(&s, nid);
            }
        });
    }

    fn deliver(s: &Shared, nid: String) {
        if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_some() {
            return;
        }
        let state = relock(&s.state);
        let Some(summary) = Self::summary(&state, &nid) else {
            return;
        };
        let sound = state.settings.notification_sound;
        drop(state);
        notifications::send(notifications::DesktopAlert {
            paths: s.catalog_paths.clone().unwrap_or_else(|| s.paths.clone()),
            summary,
            notice: nid,
            sound,
        });
    }

    fn summary(state: &State, nid: &str) -> Option<String> {
        state
            .notifications
            .iter()
            .find(|n| n.id == nid)
            .map(|n| format!("Agent: {}", n.state.label()))
            .or_else(|| {
                state
                    .terminal_notices
                    .iter()
                    .find(|n| n.id == nid && !n.dismissed)
                    .map(|n| format!("Terminal: {}", n.title))
            })
    }
}
