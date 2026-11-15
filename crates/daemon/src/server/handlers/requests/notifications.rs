use anyhow::{Context, Result};
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
        agent_workflow::apply(n, action, now())?;
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
            let _ = self.alerts.try_send(crate::workers::Alert::Notice(id));
        }
        Ok(Outcome::Changed)
    }

    /// An agent quota alert on the channels the GUI opted into. Display
    /// only: never an agent lifecycle event and never input to an agent.
    /// `[inbox, os, ntfy]` are the GUI's opt-in channels.
    pub(super) fn on_quota_notify(
        &self,
        session: &str,
        title: &str,
        [inbox, os, ntfy]: [bool; 3],
    ) -> Result<Outcome> {
        let title: String = title
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        let mut state = relock(&self.state);
        let id = if inbox {
            state.quota_notice(session, &title)?
        } else {
            None
        };
        let ping = ntfy
            .then(|| ntfy::Ping::quota(&state.settings, &title))
            .flatten();
        drop(state);
        if os {
            let summary = title.clone();
            let notice = id.clone().unwrap_or_default();
            let _ = self
                .alerts
                .try_send(crate::workers::Alert::Quota { summary, notice });
        }
        if let Some(ping) = ping
            && relock(&self.ntfy_cooldown).allow(&format!("quota:{title}"), now())
        {
            let _ = self.ntfy.try_send(ntfy::Job::Ping(ping));
        }
        Ok(if id.is_some() {
            Outcome::Changed
        } else {
            Outcome::Reply(Response::Ok)
        })
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
        self.label_agent_run(&event);
        let should_os = relock(&self.state)
            .settings
            .os_events
            .contains(&event.state);
        let mut state = relock(&self.state);
        let ping = ntfy::Ping::from_event(&state.settings, &event);
        let key = format!("{}:{}", event.agent_invocation_id, event.state as u8);
        let turn = (
            event.terminal_session_id.clone(),
            event.agent_invocation_id.clone(),
            event.agent_kind.clone(),
            event.state,
        );
        let revision = state.revision;
        let nid = state.apply_hook(event)?;
        // A duplicate or out-of-order event changes nothing, turns included.
        let applied = state.revision != revision;
        drop(state);
        if applied {
            self.mark_turn(&turn.0, (&turn.1, &turn.2), turn.3);
        }
        if nid.is_some()
            && let Some(ping) = ping
            && relock(&self.ntfy_cooldown).allow(&key, now())
        {
            let _ = self.ntfy.try_send(ntfy::Job::Ping(ping));
        }
        self.alert_unfocused(nid, should_os);
        Ok(Outcome::Deferred)
    }

    /// Usage only (`agent-usage-statusline-v1`): no turn, notice, alert or
    /// ntfy push, and no lifecycle change.
    pub(super) fn on_hook_usage(&self, event: &HookEvent) -> Result<Outcome> {
        let changed = relock(&self.state).apply_hook_usage(event)?;
        Ok(if changed {
            Outcome::Deferred
        } else {
            Outcome::Reply(Response::Ok)
        })
    }

    /// Queues the token for the ntfy worker, which writes the OS credential
    /// store; this request never waits for it.
    pub(super) fn on_ntfy_token(&self, token: String) -> Result<Outcome> {
        let token = zeroize::Zeroizing::new(token);
        anyhow::ensure!(
            token.is_empty() || crate::ntfy_token::valid(&token),
            "An ntfy token is 1–256 printable characters without spaces or quotes"
        );
        self.ntfy
            .try_send(ntfy::Job::Token(token))
            .map_err(|_| anyhow::anyhow!("The ntfy worker is busy; save the token again"))?;
        Ok(Outcome::Reply(Response::Ok))
    }

    /// Raises an OS alert unless the GUI reported focus within the last 4 s.
    fn alert_unfocused(&self, nid: Option<String>, should_os: bool) {
        let focused = *relock(&self.focused);
        if let Some(nid) = nid
            && should_os
            && (!focused.0 || focused.1.elapsed() > Duration::from_secs(4))
        {
            let _ = self.alerts.try_send(crate::workers::Alert::Notice(nid));
        }
    }
}
