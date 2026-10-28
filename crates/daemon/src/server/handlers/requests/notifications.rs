use anyhow::{Context, Result, bail};
use std::time::Duration;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;
use crate::ntfy;

impl Shared {
    pub(super) fn on_notice(&self, id: &str, action: &str) -> Result<Outcome> {
        let mut s = relock(&self.state);
        let n = s
            .notifications
            .iter_mut()
            .find(|n| n.id == id)
            .context("Unknown notification")?;
        match action {
            "read" => n.read = true,
            "dismiss" => n.dismissed = true,
            "snooze" => n.snoozed_until = now().saturating_add(600),
            _ => bail!("Unknown notification action"),
        }
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_terminal_notify(
        &self,
        session: &str,
        title: &str,
        body: &str,
    ) -> Result<Outcome> {
        let mut state = relock(&self.state);
        let os = state.settings.terminal_notifications_os;
        let id = state.terminal_notice(session, title, body)?;
        drop(state);
        if os && let Some(id) = id {
            let _ = self.alerts.try_send(id);
        }
        Ok(Outcome::Changed)
    }

    pub(super) fn on_dismiss_terminal_notice(&self, id: &str) -> Result<Outcome> {
        let mut state = relock(&self.state);
        let notice = state
            .terminal_notices
            .iter_mut()
            .find(|n| n.id == id)
            .context("Unknown terminal notice")?;
        notice.dismissed = true;
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_hook(&self, event: HookEvent) -> Result<Outcome> {
        let _operation = relock(&self.terminal_operations);
        let should_os = relock(&self.state)
            .settings
            .os_events
            .contains(&event.state);
        let mut state = relock(&self.state);
        let ping = ntfy::Ping::from_event(&state.settings, &event);
        let key = format!("{}:{}", event.agent_invocation_id, event.state as u8);
        let nid = state.apply_hook(event)?;
        drop(state);
        if nid.is_some()
            && let Some(ping) = ping
            && relock(&self.ntfy_cooldown).allow(&key, now())
        {
            let _ = self.ntfy.try_send(ping);
        }
        self.alert_unfocused(nid, should_os);
        Ok(Outcome::Changed)
    }

    /// Raises an OS alert unless the GUI reported focus within the last 4 s.
    fn alert_unfocused(&self, nid: Option<String>, should_os: bool) {
        let focused = *relock(&self.focused);
        if let Some(nid) = nid
            && should_os
            && (!focused.0 || focused.1.elapsed() > Duration::from_secs(4))
        {
            let _ = self.alerts.try_send(nid);
        }
    }
}
