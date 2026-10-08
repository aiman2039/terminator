use crate::{AgentState, Context, Lifecycle, Paths, Request, Response, Result, State, fs};
use rusqlite::{Connection, OpenFlags};

use super::catalog::{Catalog, Generation, Health, Status, coordinate};
use super::ownership::{claim_dead_owner, owner_is_serving};

pub fn clear_owned(state: &mut State) {
    state.sessions.clear();
    state.agents.clear();
    state.notifications.clear();
    state.terminal_notices.clear();
    state.presence.clear();
    state.recent_events.clear();
    state.generations.clear();
}

/// Read historical records only; a failed connection never changes their lifecycle.
/// Presence observations are live-only and never part of historical fallback.
pub fn saved(paths: &Paths) -> Result<State> {
    let conn = Connection::open_with_flags(
        paths.data.join("state.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let json: String = conn.query_row("SELECT json FROM app_state WHERE id=1", [], |r| r.get(0))?;
    let mut state: State = serde_json::from_str(&json)?;
    // Historical reads only feed display and aggregation: drop live-only
    // presence/event-dedup data and cap transient notifications/agents.
    state.compact_history();
    Ok(state)
}

/// Lock release plus an absent recorded process proves death. PID reuse is
/// conservatively treated as unavailable rather than declaring ownership lost.
#[must_use]
pub fn historical(owner: &Generation, active: Option<&str>) -> bool {
    owner.status == Status::Retired && active != Some(owner.id.as_str())
}

/// True when an owner's presence observations may merge into an aggregate
/// snapshot: the owner is live (not historical fallback, no health error)
/// and advertises the presence capability. Shared by the synchronous and
/// asynchronous generation clients.
#[must_use]
pub fn mergeable_presence(
    owner: &Generation,
    capabilities: &[String],
    error: &Option<String>,
    active: &str,
) -> bool {
    error.is_none()
        && !historical(owner, Some(active))
        && capabilities
            .iter()
            .any(|c| c == crate::AGENT_PRESENCE_CAPABILITY)
}

pub fn recover_exited(root: &Paths, owner: &Generation) -> Result<bool> {
    let Some(pid) = owner.pid else {
        return Ok(false);
    };
    let _coordination = coordinate(root)?;
    if owner_is_serving(root, owner)? {
        return Ok(false);
    }
    let Some(_lock) = claim_dead_owner(owner, pid)? else {
        return Ok(false);
    };
    let mut state = saved(&owner.paths())?;
    for session in &mut state.sessions {
        if session.lifecycle.live() {
            session.lifecycle = Lifecycle::Interrupted;
            session.pid = None;
        }
    }
    for agent in &mut state.agents {
        if !matches!(
            agent.state,
            AgentState::Completed | AgentState::Failed | AgentState::Stopped
        ) {
            agent.state = AgentState::Unknown;
        }
    }
    state.revision = state.revision.saturating_add(1);
    let conn = Connection::open(owner.data.join("state.sqlite3"))?;
    conn.execute(
        "UPDATE app_state SET json=?1 WHERE id=1",
        [serde_json::to_string(&state)?],
    )?;
    Catalog::open(root)?.retire(&owner.id)?;
    // The exclusive owner lock and dead PID permit cleanup of this generation
    // only. Its database and history remain available to the active service.
    for entry in fs::read_dir(&owner.data)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir()
            && (name == "bin" || name.to_string_lossy().starts_with(".daemon-helper-"))
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let _ = fs::remove_file(owner.paths().socket());
    let _ = fs::remove_file(owner.paths().auth());
    Ok(true)
}

/// Number of retired generations kept for History/resume. Older retired
/// generations are deleted so their state is not loaded into every snapshot.
pub const RETIRED_RETENTION: usize = 4;

/// Delete the oldest retired generations beyond `keep`. Acquires the
/// coordination lock, so callers must not already hold it. A generation whose
/// saved state still records a live session is never removed. Returns the
/// number of generations deleted.
pub fn prune_retired(root: &Paths, keep: usize) -> Result<usize> {
    let _coordination = coordinate(root)?;
    let catalog = Catalog::open(root)?;
    let active = catalog.active()?;
    let mut retired: Vec<Generation> = catalog
        .generations()?
        .into_iter()
        .filter(|g| g.status == Status::Retired && active.as_deref() != Some(g.id.as_str()))
        .collect();
    if retired.len() <= keep {
        return Ok(0);
    }
    let doomed: Vec<Generation> = retired
        .drain(..retired.len().saturating_sub(keep))
        .collect();
    let mut removed: usize = 0;
    for owner in doomed {
        if owner.data == root.data {
            continue;
        }
        let live = saved(&owner.paths())
            .map(|state| state.sessions.iter().any(|s| s.lifecycle.live()))
            .unwrap_or(true);
        if live {
            continue;
        }
        catalog.remove(&owner.id)?;
        let _ = fs::remove_file(owner.paths().socket());
        let _ = fs::remove_file(owner.paths().auth());
        let _ = fs::remove_dir_all(&owner.data);
        if owner.runtime != root.runtime {
            let _ = fs::remove_dir_all(&owner.runtime);
        }
        removed = removed.saturating_add(1);
    }
    Ok(removed)
}

pub fn snapshot(paths: &Paths) -> Result<State> {
    let catalog = Catalog::open(paths)?;
    let active = catalog.active()?.context("No active generation")?;
    let mut aggregate = State::default();
    let mut inventories = Vec::new();
    for owner in catalog.generations()? {
        if owner.status == Status::Prepared {
            continue;
        }
        let (state, error) = if historical(&owner, Some(active.as_str())) {
            (saved(&owner.paths())?, None)
        } else {
            match crate::rpc(&owner.paths(), Request::Snapshot) {
                Ok(Response::State(s)) => (*s, None),
                result => (
                    saved(&owner.paths())?,
                    Some(format!("Owner unavailable: {result:?}")),
                ),
            }
        };
        if owner.id == active {
            aggregate = state.clone();
        }
        inventories.push((owner, state, error));
    }
    clear_owned(&mut aggregate);
    aggregate.generation = active;
    catalog.refresh(&mut aggregate)?;
    for (owner, state, error) in inventories {
        // Presence merges only from live owners advertising the capability.
        // Historical fallback and unavailable owners stay presence-free, so
        // their hook records render as unverified presence.
        let present =
            mergeable_presence(&owner, &state.capabilities, &error, &aggregate.generation);
        aggregate.generations.push(Health {
            live_sessions: state.sessions.iter().filter(|s| s.lifecycle.live()).count(),
            owner,
            revision: state.revision,
            error,
            capabilities: state.capabilities,
            helper: state.attachment_helper_executable,
        });
        aggregate.sessions.extend(state.sessions);
        aggregate.agents.extend(state.agents);
        aggregate.notifications.extend(state.notifications);
        aggregate.terminal_notices.extend(state.terminal_notices);
        if present {
            aggregate.presence.extend(state.presence);
        }
    }
    Ok(aggregate)
}
