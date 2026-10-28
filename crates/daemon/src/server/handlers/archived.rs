use anyhow::{Context, Result, bail, ensure};
use terminator_core::*;

use crate::storage;

/// A retired generation's saved state, edited in place by requests that
/// target its historical sessions.
pub(super) struct ArchivedOwner<'a> {
    pub paths: &'a Paths,
    pub root: &'a Paths,
    pub state: &'a mut State,
}

impl ArchivedOwner<'_> {
    /// Applies `request`; `Some` is a reply to send without saving.
    pub(super) fn apply(&mut self, request: Request) -> Result<Option<Response>> {
        match request {
            Request::PruneHistory { budget } => self.prune_history(budget)?,
            Request::History { session } => return self.history(&session).map(Some),
            Request::Rename { session, label } => self.rename(&session, label)?,
            Request::Focus { session } => self.state.focus(&session),
            Request::Notice { id, action } => self.notice(&id, &action)?,
            Request::DismissTerminalNotice { id } => self.dismiss_terminal_notice(&id)?,
            Request::Remove { session } => self.remove(&session)?,
            Request::ClearHistory { session } => self.clear_history(session.as_deref())?,
            _ => bail!("Operation requires a live session owner"),
        }
        Ok(None)
    }

    fn prune_history(&mut self, budget: u64) -> Result<()> {
        generations::Catalog::open(self.root)?.refresh(self.state)?;
        let ids = storage::History::new(self.paths.clone())?
            .prune_with_budget(&self.state.settings, budget)?;
        for session in &mut self.state.sessions {
            if ids.contains(&session.id) {
                session.truncated = true;
            }
        }
        Ok(())
    }

    fn history(&self, session: &str) -> Result<Response> {
        let record = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == session)
            .context("Unknown historical session")?;
        Ok(Response::Text(storage::text(self.paths, record)?))
    }

    fn rename(&mut self, session: &str, label: String) -> Result<()> {
        ensure!(label.len() <= 256, "Label too long");
        self.state
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .context("Unknown session")?
            .label = label;
        Ok(())
    }

    fn notice(&mut self, id: &str, action: &str) -> Result<()> {
        let n = self
            .state
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
        Ok(())
    }

    fn dismiss_terminal_notice(&mut self, id: &str) -> Result<()> {
        self.state
            .terminal_notices
            .iter_mut()
            .find(|n| n.id == id)
            .context("Unknown notice")?
            .dismissed = true;
        Ok(())
    }

    fn remove(&mut self, session: &str) -> Result<()> {
        storage::History::new(self.paths.clone())?.clear(Some(session), true)?;
        let state = &mut *self.state;
        state.sessions.retain(|s| s.id != session);
        state.agents.retain(|a| a.session_id != session);
        state.notifications.retain(|n| n.session_id != session);
        state.terminal_notices.retain(|n| n.session_id != session);
        Ok(())
    }

    fn clear_history(&mut self, session: Option<&str>) -> Result<()> {
        storage::History::new(self.paths.clone())?.clear(session, false)?;
        for s in &mut self.state.sessions {
            if session.is_none_or(|id| id == s.id) {
                s.truncated = true;
            }
        }
        Ok(())
    }
}
