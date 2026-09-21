//! Async transport with the same owner routing and pre-execution redirects as CLI clients.
use crate::{
    Context, Duration, Envelope, MAX_FRAME, PROTOCOL_VERSION, PathBuf, Paths, Request, Response,
    Result, SnapshotHint, State, archived_generation,
    async_service::{CancellationToken, NativePool},
    bail, ensure, generations, redirect_allowed, snapshot,
};
use futures_util::{StreamExt, stream};
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

#[derive(Clone)]
pub struct Client {
    pub paths: Paths,
    pub catalog: NativePool,
    pub cpu: NativePool,
    observations: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl Client {
    #[must_use]
    pub fn new(paths: Paths, catalog: NativePool, cpu: NativePool) -> Self {
        Self {
            paths,
            catalog,
            cpu,
            observations: Default::default(),
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
        let (active, owners) = self
            .catalog
            .run(&CancellationToken::new(), move || {
                let catalog = generations::Catalog::open(&paths)?;
                Ok((
                    catalog.active()?.context("No active generation")?,
                    catalog.generations()?,
                ))
            })
            .await?;
        let mut inventories = Vec::new();
        let mut requests = stream::iter(
            owners
                .into_iter()
                .filter(|g| g.status != generations::Status::Prepared)
                .map(|owner| {
                    let active = active.clone();
                    async move {
                        let historical = generations::historical(&owner, Some(&active));
                        let response = if historical {
                            None
                        } else {
                            Some(self.direct(owner.paths(), Request::Snapshot, None).await)
                        };
                        let (state, error) = match response {
                            Some(Ok(Response::State(state))) => (*state, None),
                            result => {
                                let paths = owner.paths();
                                let saved = self
                                    .catalog
                                    .run(&CancellationToken::new(), move || {
                                        generations::saved(&paths)
                                    })
                                    .await?;
                                (
                                    saved,
                                    if historical {
                                        None
                                    } else {
                                        Some(format!("Owner unavailable: {result:?}"))
                                    },
                                )
                            }
                        };
                        Ok::<_, anyhow::Error>((owner, state, error))
                    }
                }),
        )
        .buffer_unordered(4);
        while let Some(result) = requests.next().await {
            inventories.push(result?);
        }
        drop(requests);
        // Stable ordering preserves conditional-snapshot equality across concurrent polls.
        inventories.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        let paths = self.paths.clone();
        let mut state = self
            .catalog
            .run(&CancellationToken::new(), move || {
                let mut aggregate = inventories
                    .iter()
                    .find(|(g, _, _)| g.id == active)
                    .map(|(_, s, _)| s.clone())
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
                    aggregate.generations.push(generations::Health {
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
                }
                Ok(aggregate)
            })
            .await?;
        self.observe(&mut state);
        if hint.as_ref() == Some(&state.snapshot_hint()) {
            Ok(Response::Unchanged)
        } else {
            Ok(Response::State(Box::new(state)))
        }
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
}
