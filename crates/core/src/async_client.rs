//! Async transport with the same owner routing and pre-execution redirects as CLI clients.
use crate::{
    Context, Duration, Envelope, MAX_FRAME, PROTOCOL_VERSION, PathBuf, Paths, Request, Response,
    Result, SnapshotHint, State, archived_generation,
    async_service::{CancellationToken, NativePool},
    bail, ensure, generations, redirect_allowed, snapshot,
};
use futures_util::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

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
impl Client {
    #[must_use]
    pub fn new(paths: Paths, catalog: NativePool, cpu: NativePool) -> Self {
        Self {
            paths,
            catalog,
            cpu,
            observations: Default::default(),
            historical: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn observe(&self, state: &mut State) {
        state.client_observation = self
            .observations
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
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
                                ));
                            }
                        } else {
                            targets.push((owner.paths(), original.clone()));
                        }
                    }
                    Ok(targets)
                })
                .await?;
            let mut responses =
                stream::iter(targets.into_iter().map(|(paths, request)| async move {
                    self.direct(paths, request, None).await?.checked()
                }))
                .buffer_unordered(4);
            while let Some(result) = responses.next().await {
                result?;
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
                    generations::owner_for(&paths, &route_request)
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
            return response.checked();
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
    pub async fn snapshot(&self, hint: Option<SnapshotHint>) -> Result<Response> {
        if !self.generations().await? {
            return self
                .direct(self.paths.clone(), Request::Snapshot, hint)
                .await?
                .checked();
        }
        let paths = self.paths.clone();
        let cache = self.historical.clone();
        // One catalog read lists owners and reads the shared revision. Retired
        // owners are immutable, so their state is decoded from disk once and
        // then served from `cache`; only a retired owner's first poll touches
        // SQLite. Building the aggregate is deferred until something changed.
        let (active, catalog_revision, plans) = self
            .catalog
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
                        let mut cache = cache.lock().unwrap();
                        if let Some(state) = cache.get(&owner.id) {
                            state.clone()
                        } else {
                            // A retired generation can be pruned between this
                            // catalog read and the load; skip it rather than
                            // failing the whole snapshot.
                            let Ok(state) = generations::saved(&owner.paths()) else {
                                continue;
                            };
                            let state = Arc::new(state);
                            cache.insert(owner.id.clone(), state.clone());
                            state
                        }
                    };
                    retired.insert(owner.id.clone());
                    plans.push(Plan::Historical(owner, state));
                }
                // Bound the cache to owners that are retired right now.
                cache.lock().unwrap().retain(|id, _| retired.contains(id));
                Ok((active, catalog_revision, plans))
            })
            .await?;
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
            async move {
                let response = client.direct(owner.paths(), Request::Snapshot, None).await;
                let (state, error) = match response {
                    Ok(Response::State(state)) => (Arc::new(*state), None),
                    result => {
                        let paths = owner.paths();
                        let saved = client
                            .catalog
                            .run(&CancellationToken::new(), move || {
                                generations::saved(&paths).map(Arc::new)
                            })
                            .await?;
                        (saved, Some(format!("Owner unavailable: {result:?}")))
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
        let paths = self.paths.clone();
        let mut state = self
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
            .run(&CancellationToken::new(), move || token_paths.token())
            .await?;
        let mutation = !read_only(&request);
        let timeout = if matches!(request, Request::CloseIdleSessions { .. }) {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(3)
        };
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            auth: token,
            snapshot_chunks: matches!(request, Request::Snapshot),
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
                | Request::History { session } => session.len() <= 128,
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
        let mut socket = tokio::time::timeout(timeout, UnixStream::connect(paths.socket()))
            .await
            .context("Session daemon connection deadline")?
            .context("Session daemon unavailable")?;
        let result = tokio::time::timeout(timeout, async {
            socket
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await?;
            socket.write_all(&bytes).await?;
            let mut response = self.read_response(&mut socket).await?;
            if let Response::State(state) = &mut response {
                self.observe(state);
            }
            Ok(response)
        })
        .await
        .context("IPC response deadline exceeded")
        .and_then(|r| r);
        if mutation {
            result.map_err(|e| e.context(UncertainMutation))
        } else {
            result
        }
    }
    async fn frame(&self, socket: &mut UnixStream) -> Result<Response> {
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
    async fn read_response(&self, socket: &mut UnixStream) -> Result<Response> {
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
                bytes.len() + chunk.len() <= 256 * 1024 * 1024,
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
                    matches!(response, Response::State(_)),
                    "Expected snapshot state"
                );
                Ok(response)
            })
            .await
    }
}
#[must_use]
pub fn read_only(request: &Request) -> bool {
    matches!(
        request,
        Request::Snapshot
            | Request::History { .. }
            | Request::Screen { .. }
            | Request::EditorStatus { .. }
            | Request::WorktreeList { .. }
            | Request::ShellCommand { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_frame;
    async fn fixture() -> (tempfile::TempDir, Client, tokio::net::UnixListener) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        std::fs::write(paths.auth(), "fixture").unwrap();
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        let client = Client::new(
            paths,
            NativePool::new("catalog-test", 1).unwrap(),
            NativePool::new("cpu-test", 2).unwrap(),
        );
        (dir, client, listener)
    }
    async fn envelope(socket: &mut UnixStream) -> Envelope {
        let len = socket.read_u32().await.unwrap() as usize;
        let mut bytes = vec![0; len];
        socket.read_exact(&mut bytes).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[tokio::test]
    async fn fragmented_legacy_response_preserves_snapshot_hint_and_auth() {
        let (_dir, client, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = envelope(&mut socket).await;
            assert_eq!(request.auth, "fixture");
            assert!(request.snapshot_chunks);
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &Response::State(Box::default())).unwrap();
            for fragment in bytes.chunks(3) {
                socket.write_all(fragment).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert!(matches!(
            client.rpc(Request::Snapshot).await.unwrap(),
            Response::State(_)
        ));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn connection_loss_after_mutation_is_uncertain_and_never_replayed() {
        let (_dir, client, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            envelope(&mut socket).await;
            drop(socket);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let error = client
            .rpc(Request::Focus {
                session: "fixture".into(),
            })
            .await
            .unwrap_err();
        assert!(
            error.downcast_ref::<UncertainMutation>().is_some(),
            "{error:#}"
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn oversized_response_is_rejected_before_allocation() {
        let (_dir, client, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            envelope(&mut socket).await;
            socket.write_u32((MAX_FRAME + 1) as u32).await.unwrap();
        });
        assert!(
            client
                .rpc(Request::Snapshot)
                .await
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn catalog_snapshot_short_circuits_unchanged_without_rebuilding() {
        use crate::generations::{self, Catalog, Generation, Status};
        let dir = tempfile::Builder::new()
            .prefix("snap-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = Paths::at(dir.path().into());
        root.init().unwrap();
        generations::migrate_idle(&root).unwrap();
        let id = crate::id();
        let owner = Generation {
            id: id.clone(),
            data: root.data.join("generations").join(&id),
            runtime: root.runtime.join(&id[..8]),
            version: "test".into(),
            build: "test".into(),
            protocol: PROTOCOL_VERSION,
            catalog: generations::CATALOG_VERSION,
            status: Status::Prepared,
            pid: None,
        };
        owner.paths().init().unwrap();
        let db = rusqlite::Connection::open(owner.data.join("state.sqlite3")).unwrap();
        db.execute_batch(
            "CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;",
        )
        .unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&State {
                generation: id.clone(),
                ..Default::default()
            })
            .unwrap()],
        )
        .unwrap();
        let mut catalog = Catalog::open(&root).unwrap();
        catalog.register(&owner).unwrap();
        catalog.set_pid(&id, std::process::id()).unwrap();
        catalog.activate(&id).unwrap();
        std::fs::write(owner.paths().auth(), "fixture").unwrap();
        let served = State {
            generation: id.clone(),
            revision: 7,
            ..Default::default()
        };
        let listener = tokio::net::UnixListener::bind(owner.paths().socket()).unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let _ = envelope(&mut socket).await;
                let mut bytes = Vec::new();
                write_frame(&mut bytes, &Response::State(Box::new(served.clone()))).unwrap();
                let _ = socket.write_all(&bytes).await;
            }
        });
        let client = Client::new(
            root,
            NativePool::new("catalog-snap", 1).unwrap(),
            NativePool::new("cpu-snap", 1).unwrap(),
        );
        let Response::State(first) = client.snapshot(None).await.unwrap() else {
            panic!("expected a built state");
        };
        assert_eq!(first.revision, 7);
        assert!(matches!(
            client.snapshot(Some(first.snapshot_hint())).await.unwrap(),
            Response::Unchanged
        ));
        server.abort();
    }
}
