//! The aggregate snapshot across live and retired generations.
use super::*;
use crate::{ErrorReport, SnapshotHint};
use futures_util::{StreamExt, stream};
use std::collections::HashSet;

/// An owner, its state, and the health error its snapshot reported.
type Inventory = (generations::Generation, Arc<State>, Option<String>);
/// The active generation, catalog revision, and per-owner plans.
type Plans = (String, u64, Vec<Plan>);
type HistoricalCache = Arc<Mutex<HashMap<String, Arc<State>>>>;

struct BuildAggregateInput {
    active: String,
    inventories: Vec<Inventory>,
}

impl Client {
    pub async fn snapshot(&self, hint: Option<SnapshotHint>) -> Result<Response> {
        if !self.generations().await? {
            return Ok(self
                .direct(self.paths.clone(), Request::Snapshot, hint)
                .await?
                .checked()?);
        }
        let (active, catalog_revision, plans) = self.snapshot_plans().await?;
        let inventories = self.inventories(&active, plans).await?;
        // Compare the cheap hint before rebuilding the aggregate. The usual
        // 100 ms poll has no changes, so this skips cloning and serializing the
        // entire history. Historical states come from the cache, so unchanged
        // polls no longer re-read or JSON-decode the retired generations.
        let candidate = snapshot_hint(&active, catalog_revision, &inventories);
        if hint.as_ref() == Some(&candidate) {
            return Ok(Response::Unchanged);
        }
        let mut state = self
            .build_aggregate(BuildAggregateInput {
                active,
                inventories,
            })
            .await?;
        self.observe(&mut state);
        Ok(Response::State(Box::new(state)))
    }

    /// Every owner's state, sorted by owner ID: retired ones from the plans,
    /// live ones queried concurrently.
    async fn inventories(&self, active: &str, plans: Vec<Plan>) -> Result<Vec<Inventory>> {
        let mut inventories = Vec::with_capacity(plans.len());
        let mut live = Vec::new();
        for plan in plans {
            match plan {
                Plan::Live(owner) => live.push(owner),
                Plan::Historical(owner, state) => inventories.push((owner, state, None)),
            }
        }
        let mut responses = stream::iter(
            live.into_iter()
                .map(|owner| self.clone().live_inventory(owner, active.to_owned())),
        )
        .buffer_unordered(4);
        while let Some(result) = responses.next().await {
            inventories.push(result?);
        }
        drop(responses);
        // Stable ordering preserves conditional-snapshot equality across concurrent polls.
        inventories.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        Ok(inventories)
    }

    /// A live owner's snapshot, or its saved state with a health error when
    /// it does not answer.
    async fn live_inventory(
        self,
        owner: generations::Generation,
        active: String,
    ) -> Result<Inventory> {
        let response = self.direct(owner.paths(), Request::Snapshot, None).await;
        let (state, error) = match response {
            Ok(Response::State(state)) => (Arc::new(*state), None),
            result => {
                let error = format!(
                    "Owner unavailable: {:?}",
                    result.as_ref().map_err(|e| ErrorReport(e))
                );
                let unavailable = owner.clone();
                let (saved, error) = self
                    .catalog
                    .run(&CancellationToken::new(), move || {
                        generations::unavailable_snapshot(&unavailable, &active, error)
                            .map_err(AsyncClientError::from)
                    })
                    .await?;
                (Arc::new(saved), error)
            }
        };
        Ok((owner, state, error))
    }

    async fn snapshot_plans(&self) -> Result<Plans> {
        let paths = self.paths.clone();
        let cache = Arc::clone(&self.historical);
        // One catalog read lists owners and reads the shared revision. Retired
        // owners are immutable, so their state is decoded from disk once and
        // then served from `cache`; only a retired owner's first poll touches
        // SQLite. Building the aggregate is deferred until something changed.
        self.catalog
            .run(&CancellationToken::new(), move || plans(&paths, &cache))
            .await
    }

    async fn build_aggregate(&self, input: BuildAggregateInput) -> Result<State> {
        let BuildAggregateInput {
            active,
            inventories,
        } = input;
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                aggregate(&paths, active, inventories)
            })
            .await
    }
}

fn plans(paths: &Paths, cache: &HistoricalCache) -> Result<Plans> {
    let catalog = generations::Catalog::open(paths)?;
    let active = catalog
        .active()?
        .ok_or(generations::GenerationsError::NoActiveGeneration)?;
    let catalog_revision = catalog.revision()?;
    let mut plans = Vec::new();
    let mut retired = HashSet::new();
    for owner in catalog.generations()? {
        if owner.status == generations::Status::Prepared {
            continue;
        }
        if !generations::historical(&owner, Some(active.as_str())) {
            plans.push(Plan::Live(owner));
            continue;
        }
        let Some(state) = cached_state(cache, &owner) else {
            continue;
        };
        retired.insert(owner.id.clone());
        plans.push(Plan::Historical(owner, state));
    }
    // Bound the cache to owners that are retired right now.
    cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|id, _| retired.contains(id));
    Ok((active, catalog_revision, plans))
}

/// A retired owner's state, read from disk on first use. `None` when it was
/// pruned between the catalog read and this load; the snapshot skips it
/// rather than failing.
fn cached_state(cache: &HistoricalCache, owner: &generations::Generation) -> Option<Arc<State>> {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(state) = cache.get(&owner.id) {
        return Some(Arc::clone(state));
    }
    let state = Arc::new(generations::saved(&owner.paths()).ok()?);
    cache.insert(owner.id.clone(), Arc::clone(&state));
    Some(state)
}

fn snapshot_hint(active: &str, catalog_revision: u64, inventories: &[Inventory]) -> SnapshotHint {
    let active_revision = inventories
        .iter()
        .find(|(owner, _, _)| owner.id == active)
        .map_or(0, |(_, state, _)| state.revision);
    let owner_revisions = inventories
        .iter()
        .map(|(owner, state, error)| {
            (
                owner.id.clone(),
                state.revision,
                error.clone(),
                owner.status.clone(),
            )
        })
        .collect();
    SnapshotHint {
        generation: active.to_owned(),
        revision: active_revision,
        catalog_revision,
        owner_revisions,
    }
}

fn aggregate(paths: &Paths, active: String, inventories: Vec<Inventory>) -> Result<State> {
    let mut aggregate = inventories
        .iter()
        .find(|(g, _, _)| g.id == active)
        .map(|(_, s, _)| (**s).clone())
        .unwrap_or_default();
    generations::clear_owned(&mut aggregate);
    let catalog = generations::Catalog::open(paths)?;
    if catalog.active()?.as_deref() != Some(active.as_str()) {
        return Err(AsyncClientError::Superseded { generation: active });
    }
    aggregate.generation = active;
    catalog.refresh(&mut aggregate)?;
    for inventory in inventories {
        merge_inventory(&mut aggregate, inventory);
    }
    Ok(aggregate)
}

fn merge_inventory(aggregate: &mut State, (owner, state, error): Inventory) {
    // Presence merges only from live owners advertising the capability;
    // historical and unavailable owners stay presence-free so hook records
    // render as unverified.
    let present = generations::mergeable_presence(generations::MergeablePresenceInput {
        owner: &owner,
        capabilities: &state.capabilities,
        error: &error,
        active: &aggregate.generation,
    });
    aggregate.generations.push(generations::Health {
        live_sessions: state.sessions.iter().filter(|s| s.lifecycle.live()).count(),
        owner,
        revision: state.revision,
        error,
        capabilities: state.capabilities.clone(),
        helper: state.attachment_helper_executable.clone(),
    });
    aggregate.sessions.extend(state.sessions.iter().cloned());
    aggregate
        .vnc_sessions
        .extend(state.vnc_sessions.iter().cloned());
    aggregate.agents.extend(state.agents.iter().cloned());
    aggregate
        .notifications
        .extend(state.notifications.iter().cloned());
    aggregate
        .terminal_notices
        .extend(state.terminal_notices.iter().cloned());
    if present {
        aggregate.presence.extend(state.presence.iter().cloned());
    }
}
