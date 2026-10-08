use anyhow::{Context, Result, anyhow, bail, ensure};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use terminator_core::*;

use super::super::shared::{HistoryJob, Runtime, Shared, relock};
use crate::storage;

impl Shared {
    pub(crate) fn forward_input(&self, runtime: &Arc<Mutex<Runtime>>, bytes: &[u8]) -> Result<()> {
        let writer = {
            let _operation = relock(&self.terminal_operations);
            let mut runtime = relock(runtime);
            ensure!(!runtime.ended && !runtime.closing, "Session is closing");
            runtime.inputs_in_flight = runtime.inputs_in_flight.saturating_add(1);
            Arc::clone(&runtime.writer)
        };
        // A full PTY input buffer must not block other panes or Stop.
        // In-flight input makes idle-close ineligible until it finishes.
        let result = {
            let mut writer = relock(&writer);
            writer.write_all(bytes).and_then(|()| writer.flush())
        };
        {
            let _operation = relock(&self.terminal_operations);
            let mut runtime = relock(runtime);
            runtime.inputs_in_flight = runtime.inputs_in_flight.saturating_sub(1);
        }
        result.map_err(Into::into)
    }

    pub(crate) fn prune_global(self: &Arc<Self>, owners: &[generations::Generation]) -> Result<()> {
        let mut segments = Vec::new();
        let mut budgets = HashMap::<String, u64>::new();
        for owner in owners
            .iter()
            .filter(|g| g.status != generations::Status::Prepared)
        {
            budgets.insert(owner.id.clone(), 0);
            for path in storage::history_files(&owner.paths(), None) {
                let metadata = fs::metadata(&path)?;
                let usage = budgets
                    .get_mut(&owner.id)
                    .ok_or_else(|| anyhow!("Missing history budget"))?;
                *usage = usage
                    .checked_add(metadata.len())
                    .ok_or_else(|| anyhow!("History budget overflow"))?;
                segments.push((metadata.modified()?, owner.id.clone(), metadata.len()));
            }
        }
        segments.sort_by_key(|s| s.0);
        let total_mib = relock(&self.state).settings.total_mib;
        let limit = total_mib
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
            .ok_or_else(|| anyhow!("History budget overflow"))?;
        let original_budgets = budgets.clone();
        let mut total: u64 = budgets.values().sum();
        for (_, owner, bytes) in segments {
            if total <= limit {
                break;
            }
            total = total.saturating_sub(bytes);
            let budget = budgets
                .get_mut(&owner)
                .ok_or_else(|| anyhow!("Missing history budget"))?;
            *budget = budget
                .checked_sub(bytes)
                .ok_or_else(|| anyhow!("History budget underflow"))?;
        }
        let mine = relock(&self.state).generation.clone();
        for owner in owners
            .iter()
            .filter(|g| g.status != generations::Status::Prepared)
        {
            let budget = budgets
                .get(&owner.id)
                .copied()
                .ok_or_else(|| anyhow!("Missing history budget"))?;
            let original = original_budgets
                .get(&owner.id)
                .copied()
                .ok_or_else(|| anyhow!("Missing history budget"))?;
            let reduced = budget < original;
            if !reduced && owner.status != generations::Status::Retired {
                continue;
            }
            let request = Request::PruneHistory {
                budget: if reduced { budget } else { limit },
            };
            self.prune_owner_history(owner, request, &mine)?;
        }
        Ok(())
    }

    pub(crate) fn prune_owner_history(
        self: &Arc<Self>,
        owner: &generations::Generation,
        request: Request,
        mine: &str,
    ) -> Result<()> {
        if owner.status != generations::Status::Retired || owner.id == mine {
            if owner.id == mine {
                self.handle(request)?;
            } else {
                rpc(&owner.paths(), request)?;
            }
            return Ok(());
        }
        if generations::saved(&owner.paths())?
            .sessions
            .iter()
            .any(|s| s.lifecycle.live())
        {
            return Ok(());
        }
        self.handle(Request::Archived {
            generation: owner.id.clone(),
            request: Box::new(request),
        })?;
        Ok(())
    }

    pub(crate) fn handle_archived(
        self: &Arc<Self>,
        generation: String,
        request: Box<Request>,
    ) -> Result<Response> {
        let root = self
            .catalog_paths
            .as_ref()
            .context("No generation catalog")?;
        let _coordination = generations::coordinate(root)?;
        let catalog = generations::Catalog::open(root)?;
        if catalog.active()?.as_deref() == Some(generation.as_str()) {
            drop(_coordination);
            return self.handle(*request);
        }
        let owner = catalog
            .generations()?
            .into_iter()
            .find(|g| g.id == generation && g.status == generations::Status::Retired)
            .context("Owner is not retired")?;
        let paths = owner.paths();
        let (store, mut state) = storage::Store::open(&paths)?;
        ensure!(
            !state.sessions.iter().any(|s| s.lifecycle.live()),
            "Retired owner still has live records"
        );
        match *request {
            Request::PruneHistory { budget } => {
                generations::Catalog::open(root)?.refresh(&mut state)?;
                let ids =
                    storage::History::new(paths)?.prune_with_budget(&state.settings, budget)?;
                for session in &mut state.sessions {
                    if ids.contains(&session.id) {
                        session.truncated = true;
                    }
                }
            }
            Request::History { session } => {
                let record = state
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .context("Unknown historical session")?;
                return Ok(Response::Text(storage::text(&paths, record)?));
            }
            Request::Rename { session, label } => {
                ensure!(label.len() <= 256, "Label too long");
                state
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == session)
                    .context("Unknown session")?
                    .label = label;
            }
            Request::Focus { session } => state.focus(&session),
            Request::Notice { id, action } => {
                let n = state
                    .notifications
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notification")?;
                match action.as_str() {
                    "read" => n.read = true,
                    "dismiss" => n.dismissed = true,
                    "snooze" => n.snoozed_until = now().saturating_add(600),
                    _ => bail!("Unknown notification action"),
                }
            }
            Request::DismissTerminalNotice { id } => {
                state
                    .terminal_notices
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notice")?
                    .dismissed = true;
            }
            Request::Remove { session } => {
                storage::History::new(paths)?.clear(Some(&session), true)?;
                state.sessions.retain(|s| s.id != session);
                state.agents.retain(|a| a.session_id != session);
                state.notifications.retain(|n| n.session_id != session);
                state.terminal_notices.retain(|n| n.session_id != session);
            }
            Request::ClearHistory { session } => {
                storage::History::new(paths)?.clear(session.as_deref(), false)?;
                for s in &mut state.sessions {
                    if session.as_ref().is_none_or(|id| id == &s.id) {
                        s.truncated = true;
                    }
                }
            }
            _ => bail!("Operation requires a live session owner"),
        }
        state.revision = state.revision.saturating_add(1);
        store.save(&state)?;
        Ok(Response::Ok)
    }

    pub(crate) fn history_clear(&self, session: Option<String>, remove: bool) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.history.send(HistoryJob::Clear(session, remove, tx))?;
        rx.recv_timeout(Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)
    }
    pub(crate) fn persist(&self) -> Result<()> {
        let s = relock(&self.state).clone();
        relock(&self.store).save(&s)
    }
    pub(crate) fn runtime(&self, id: &str) -> Result<Arc<Mutex<Runtime>>> {
        relock(&self.sessions)
            .get(id)
            .cloned()
            .context("Session is not live")
    }
    pub(crate) fn degraded(&self, message: &str) {
        let mut s = relock(&self.state);
        s.degraded = Some(message.into());
        s.revision = s.revision.saturating_add(1);
    }
}
