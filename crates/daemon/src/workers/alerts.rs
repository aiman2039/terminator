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

/// One desktop alert: a stored notice, or an agent quota alert whose user
/// did not want it in the Agents inbox.
pub(crate) enum Alert {
    Notice(String),
    Quota { summary: String, notice: String },
}

impl AlertWorker {
    pub(crate) fn spawn(weak: Weak<Shared>, alerts: mpsc::Receiver<Alert>) {
        thread::spawn(move || {
            while let Ok(alert) = alerts.recv() {
                let Some(s) = weak.upgrade() else { break };
                Self::deliver(&s, alert);
            }
        });
    }

    fn deliver(s: &Shared, alert: Alert) {
        if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_some() {
            return;
        }
        let state = relock(&s.state);
        let (summary, nid) = match alert {
            Alert::Notice(nid) => match Self::summary(&state, &nid) {
                Some(summary) => (summary, nid),
                None => return,
            },
            Alert::Quota { summary, notice } => (summary, notice),
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
