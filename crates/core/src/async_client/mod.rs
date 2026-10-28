//! Async transport with the same owner routing and pre-execution redirects as CLI clients.
use crate::transport::AsyncStream;
use crate::{
    Context, Envelope, MAX_FRAME, PROTOCOL_VERSION, PathBuf, Paths, Request, Response, Result,
    SnapshotHint, State,
    async_service::{CancellationToken, NativePool},
    bail, ensure, generations,
    ipc::{archived_generation, redirect_allowed},
    snapshot,
};
use futures_util::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
pub struct UncertainMutation;
impl std::fmt::Display for UncertainMutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mutation outcome is uncertain; reconcile state before retrying")
    }
}
impl std::error::Error for UncertainMutation {}

/// A snapshot owner, resolved up front so retired owners can serve a cached
/// state while live owners are queried over their sockets.
enum Plan {
    Live(generations::Generation),
    Historical(generations::Generation, Arc<State>),
}

#[derive(Clone)]
pub struct Client {
    pub paths: Paths,
    pub catalog: NativePool,
    pub cpu: NativePool,
    observations: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Retired generations are immutable: their state is parsed from disk once
    /// and reused. Without this, every 100 ms poll re-read and JSON-decoded the
    /// whole history (tens of megabytes) for each retired owner.
    historical: Arc<Mutex<HashMap<String, Arc<State>>>>,
}
struct BuildAggregateInput {
    active: String,
    inventories: Vec<(generations::Generation, Arc<State>, Option<String>)>,
}

impl Client {
    #[must_use]
    pub fn new(paths: Paths, catalog: NativePool, cpu: NativePool) -> Self {
        Self {
            paths,
            catalog,
            cpu,
            observations: Arc::default(),
            historical: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn observe(&self, state: &mut State) {
        state.client_observation = self
            .observations
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            .saturating_add(1);
    }
    /// Drop one retired owner's cached snapshot so the next poll re-reads it
    /// from disk. Retired state is otherwise served from [`Self::historical`]
    /// for the life of the client, which would resurrect a just-dismissed,
    /// read, or snoozed historical notice on the next full snapshot.
    fn evict_historical(&self, generation: &str) {
        self.historical
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(generation);
    }
    async fn generations(&self) -> Result<bool> {
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                Ok(generations::exists(&paths))
            })
            .await
    }
    pub async fn rpc(&self, request: Request) -> Result<Response> {
        if !self.generations().await? {
            return self
                .direct(self.paths.clone(), request, None)
                .await?
                .checked();
        }
        if matches!(request, Request::Snapshot) {
            return self.snapshot(None).await;
        }
        if matches!(
            request,
            Request::Heartbeat { .. } | Request::ClearHistory { session: None }
        ) {
            let paths = self.paths.clone();
            let original = request.clone();
            let targets = self
                .catalog
                .run(&CancellationToken::new(), move || {
                    let catalog = generations::Catalog::open(&paths)?;
                    let active = catalog.active()?;
                    let mut targets = Vec::new();
                    for owner in catalog.generations()? {
                        if owner.status == generations::Status::Prepared {
                            continue;
                        }
                        if generations::historical(&owner, active.as_deref()) {
                            if matches!(original, Request::ClearHistory { .. }) {
                                targets.push((
                                    generations::owner_for(&paths, &Request::Snapshot)?,
                                    Request::Archived {
                                        generation: owner.id,
                                        request: Box::new(original.clone()),
                                    },
                                    true,
                                ));
                            }
                        } else {
                            let required = !matches!(original, Request::Heartbeat { .. })
                                || active.as_deref() == Some(owner.id.as_str());
                            targets.push((owner.paths(), original.clone(), required));
                        }
                    }
                    Ok(targets)
                })
                .await?;
            let retired = targets
                .iter()
                .filter_map(|(_, request, _)| archived_mutation(request))
                .map(str::to_string)
                .collect::<Vec<_>>();
            let mut responses = stream::iter(targets.into_iter().map(
                |(paths, request, required)| async move {
                    let result = match self.direct(paths, request, None).await {
                        Ok(response) => response.checked(),
                        Err(error) => Err(error),
                    };
                    (required, result)
                },
            ))
            .buffer_unordered(4);
            while let Some((required, result)) = responses.next().await {
                // Snapshot retains unavailable-owner diagnostics. Heartbeats
                // tolerate only failures of non-active owners; real mutations
                // and active-owner failures must remain visible to callers.
                if required {
                    result?;
                }
            }
            for generation in retired {
                self.evict_historical(&generation);
            }
            return Ok(Response::Ok);
        }
        let paths = self.paths.clone();
        let request = self
            .catalog
            .run(&CancellationToken::new(), move || {
                if let Some(owner) = archived_generation(&paths, &request)?
                    && !matches!(
                        request,
                        Request::Archived { .. } | Request::Shutdown | Request::ShutdownIfIdle
                    )
                {
                    return Ok(Request::Archived {
                        generation: owner,
                        request: Box::new(request),
                    });
                }
                Ok(request)
            })
            .await?;
        for _ in 0..3 {
            let paths = self.paths.clone();
            let route_request = request.clone();
            let target = self
                .catalog
                .run(&CancellationToken::new(), move || {
                    Ok(generations::owner_for(&paths, &route_request)?)
                })
                .await?;
            let response = self.direct(target, request.clone(), None).await?;
            if let Response::Redirect { generation } = &response
                && redirect_allowed(&request)
            {
                let generation = generation.clone();
                let paths = self.paths.clone();
                self.catalog
                    .run(&CancellationToken::new(), move || {
                        ensure!(
                            generations::Catalog::open(&paths)?
                                .generations()?
                                .iter()
                                .any(|g| g.id == generation),
                            "Redirect names an unregistered owner"
                        );
                        Ok(())
                    })
                    .await?;
                continue;
            }
            let response = response.checked()?;
            if let Some(generation) = archived_mutation(&request) {
                self.evict_historical(generation);
            }
            return Ok(response);
        }
        bail!("Active service changed repeatedly before execution; retry the operation")
    }
    pub async fn editor_socket(&self, session: String) -> Result<PathBuf> {
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                Ok(generations::owner_for(
                    &paths,
                    &Request::EditorStatus {
                        session: session.clone(),
                    },
                )?
                .editor_socket(&session))
            })
            .await
    }
    async fn snapshot_plans(&self) -> Result<(String, u64, Vec<Plan>)> {
        let paths = self.paths.clone();
        let cache = Arc::clone(&self.historical);
        // One catalog read lists owners and reads the shared revision. Retired
        // owners are immutable, so their state is decoded from disk once and
        // then served from `cache`; only a retired owner's first poll touches
        // SQLite. Building the aggregate is deferred until something changed.
        self.catalog
            .run(&CancellationToken::new(), move || {
                let catalog = generations::Catalog::open(&paths)?;
                let active = catalog.active()?.context("No active generation")?;
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
                    let state = {
                        let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
                        if let Some(state) = cache.get(&owner.id) {
                            Arc::clone(state)
                        } else {
                            // A retired generation can be pruned between this
                            // catalog read and the load; skip it rather than
                            // failing the whole snapshot.
                            let Ok(state) = generations::saved(&owner.paths()) else {
                                continue;
                            };
                            let state = Arc::new(state);
                            cache.insert(owner.id.clone(), Arc::clone(&state));
                            state
                        }
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
            })
            .await
    }

    async fn build_aggregate(&self, input: BuildAggregateInput) -> Result<State> {
        let BuildAggregateInput {
            active,
            inventories,
        } = input;
        let paths = self.paths.clone();
        let state = self
            .catalog
            .run(&CancellationToken::new(), move || {
                let mut aggregate = inventories
                    .iter()
                    .find(|(g, _, _)| g.id == active)
                    .map(|(_, s, _)| (**s).clone())
                    .unwrap_or_default();
                generations::clear_owned(&mut aggregate);
                let catalog = generations::Catalog::open(&paths)?;
                ensure!(
                    catalog.active()?.as_deref() == Some(active.as_str()),
                    "Snapshot superseded by an active-owner change"
                );
                aggregate.generation = active;
                catalog.refresh(&mut aggregate)?;
                for (owner, state, error) in inventories {
                    // Presence merges only from live owners advertising the
                    // capability; historical and unavailable owners stay
                    // presence-free so hook records render as unverified.
                    let present = generations::mergeable_presence(
                        &owner,
                        &state.capabilities,
                        &error,
                        &aggregate.generation,
                    );
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
                Ok(aggregate)
            })
            .await?;
        Ok(state)
    }

    pub async fn snapshot(&self, hint: Option<SnapshotHint>) -> Result<Response> {
        if !self.generations().await? {
            return self
                .direct(self.paths.clone(), Request::Snapshot, hint)
                .await?
                .checked();
        }
        let (active, catalog_revision, plans) = self.snapshot_plans().await?;
        let mut inventories = Vec::with_capacity(plans.len());
        let mut live = Vec::new();
        for plan in plans {
            match plan {
                Plan::Live(owner) => live.push(owner),
                Plan::Historical(owner, state) => inventories.push((owner, state, None)),
            }
        }
        let mut responses = stream::iter(live.into_iter().map(|owner| {
            let client = self.clone();
            let active = active.clone();
            async move {
                let response = client.direct(owner.paths(), Request::Snapshot, None).await;
                let (state, error) = match response {
                    Ok(Response::State(state)) => (Arc::new(*state), None),
                    result => {
                        let unavailable = owner.clone();
                        let (saved, error) = client
                            .catalog
                            .run(&CancellationToken::new(), move || {
                                Ok(generations::unavailable_snapshot(
                                    &unavailable,
                                    &active,
                                    format!("Owner unavailable: {result:?}"),
                                )?)
                            })
                            .await?;
                        (Arc::new(saved), error)
                    }
                };
                Ok::<_, anyhow::Error>((owner, state, error))
            }
        }))
        .buffer_unordered(4);
        while let Some(result) = responses.next().await {
            inventories.push(result?);
        }
        drop(responses);
        // Stable ordering preserves conditional-snapshot equality across concurrent polls.
        inventories.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        // Compare the cheap hint before rebuilding the aggregate. The usual
        // 100 ms poll has no changes, so this skips cloning and serializing the
        // entire history. Historical states come from the cache, so unchanged
        // polls no longer re-read or JSON-decode the retired generations.
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
        let candidate = SnapshotHint {
            generation: active.clone(),
            revision: active_revision,
            catalog_revision,
            owner_revisions,
        };
        if hint.as_ref() == Some(&candidate) {
            return Ok(Response::Unchanged);
        }
        let mut state = self
            .build_aggregate(BuildAggregateInput {
                active: active.clone(),
                inventories,
            })
            .await?;
        self.observe(&mut state);
        Ok(Response::State(Box::new(state)))
    }
    pub async fn direct(
        &self,
        paths: Paths,
        request: Request,
        hint: Option<SnapshotHint>,
    ) -> Result<Response> {
        let token_paths = paths.clone();
        let token = self
            .catalog
            .run(&CancellationToken::new(), move || Ok(token_paths.token()?))
            .await?;
        let mutation = !read_only(&request);
        // Static operation name for the deadline below; the envelope takes
        // ownership of the request afterwards.
        let operation = request.name();
        let timeout = request.response_deadline();
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            auth: token,
            snapshot_chunks: matches!(request, Request::Snapshot | Request::VncFrame { .. }),
            ssh_errors: true,
            request,
            snapshot_hint: hint,
        };
        let inline = envelope.auth.len() <= 4096
            && match &envelope.request {
                Request::Snapshot | Request::Heartbeat { .. } => true,
                Request::Focus { session }
                | Request::Stop { session }
                | Request::EditorSave { session }
                | Request::EditorStatus { session }
                | Request::Screen { session }
                | Request::History { session }
                | Request::VncFrame { session, .. }
                | Request::VncDisconnect { session } => session.len() <= 128,
                Request::VncInput { session, events } => session.len() <= 128 && events.len() <= 32,
                _ => false,
            };
        let encode = move || -> Result<Vec<u8>> {
            let bytes = serde_json::to_vec(&envelope)?;
            ensure!(bytes.len() <= MAX_FRAME, "IPC frame too large");
            Ok(bytes)
        };
        // Tiny protocol messages must not queue behind noninterruptible image/diff work.
        let bytes = if inline {
            encode()?
        } else {
            self.cpu.run(&CancellationToken::new(), encode).await?
        };
        let mut socket = tokio::time::timeout(timeout, AsyncStream::connect_ipc(&paths.socket()))
            .await
            .context("Session daemon connection deadline")?
            .context("Session daemon unavailable")?;
        let result = tokio::time::timeout(timeout, async {
            let len = u32::try_from(bytes.len()).context("IPC frame too large")?;
            socket.write_all(&len.to_be_bytes()).await?;
            socket.write_all(&bytes).await?;
            let mut response = self.read_response(&mut socket).await?;
            if let Response::State(state) = &mut response {
                self.observe(state);
            }
            Ok(response)
        })
        .await
        .with_context(|| format!("IPC response deadline exceeded for {operation}"))
        .and_then(|r| r);
        if mutation {
            result.map_err(|e| e.context(UncertainMutation))
        } else {
            result
        }
    }
    async fn frame(&self, socket: &mut AsyncStream) -> Result<Response> {
        let n = socket.read_u32().await? as usize;
        ensure!(n <= MAX_FRAME, "IPC frame too large");
        let mut bytes = vec![0; n];
        socket.read_exact(&mut bytes).await?;
        if bytes.len() <= 64 * 1024 {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        self.cpu
            .run(&CancellationToken::new(), move || {
                Ok(serde_json::from_slice(&bytes)?)
            })
            .await
    }
    async fn read_response(&self, socket: &mut AsyncStream) -> Result<Response> {
        let mut response = self.frame(socket).await?;
        if !matches!(response, Response::SnapshotChunk { .. }) {
            return Ok(response);
        }
        let mut bytes = Vec::new();
        loop {
            let Response::SnapshotChunk { data, last } = response else {
                bail!("Interrupted snapshot transfer")
            };
            let chunk = self
                .cpu
                .run(&CancellationToken::new(), move || snapshot::decode(&data))
                .await?;
            ensure!(
                bytes
                    .len()
                    .checked_add(chunk.len())
                    .is_some_and(|total| total <= snapshot::MAX_RESPONSE_BYTES),
                "Snapshot exceeds the 256 MiB client limit; saved state is intact"
            );
            bytes.extend(chunk);
            if last {
                break;
            }
            response = self.frame(socket).await?;
        }
        self.cpu
            .run(&CancellationToken::new(), move || {
                let response = serde_json::from_slice(&bytes)?;
                ensure!(
                    matches!(response, Response::State(_) | Response::VncFrame(_)),
                    "Expected snapshot state or VNC frame"
                );
                Ok(response)
            })
            .await
    }
}
/// The retired generation an archived request mutates, if any. Reads served
/// from a retired owner's disk state (history text) leave the snapshot cache
/// valid; everything else the retired owner accepts changes what the next
/// snapshot must re-read.
fn archived_mutation(request: &Request) -> Option<&str> {
    if let Request::Archived {
        generation,
        request,
    } = request
        && !matches!(**request, Request::History { .. })
    {
        return Some(generation);
    }
    None
}

#[must_use]
pub fn read_only(request: &Request) -> bool {
    if let Request::Archived { request, .. } = request {
        // A retired owner's disk reads (history text) change nothing; only
        // retired writes must evict the snapshot cache. See
        // [`archived_mutation`].
        return read_only(request);
    }
    if let Request::RemoteProject { operation, .. } = request {
        return !operation.starts_transfer();
    }
    matches!(
        request,
        Request::ProbeRemoteProject { .. }
            | Request::ScanHostKey { .. }
            | Request::ForwardStatus
            | Request::Snapshot
            | Request::History { .. }
            | Request::Screen { .. }
            | Request::EditorStatus { .. }
            | Request::WorktreeList { .. }
            | Request::ShellCommand { .. }
            | Request::VncFrame { .. }
    )
}

#[cfg(test)]
mod tests;
