//! Async transport with the same owner routing and pre-execution redirects as CLI clients.
use crate::{
    IpcError, PathBuf, Paths, Request, Response, State,
    async_service::{CancellationToken, NativePool},
    generations,
    ipc::{archived_generation, redirect_allowed},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

mod direct;
mod error;
mod routing;
mod snapshot;
pub use error::AsyncClientError;
type Result<T, E = AsyncClientError> = std::result::Result<T, E>;

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
            return Ok(self
                .direct(self.paths.clone(), request, None)
                .await?
                .checked()?);
        }
        if matches!(request, Request::Snapshot) {
            return self.snapshot(None).await;
        }
        if matches!(
            request,
            Request::Heartbeat { .. } | Request::ClearHistory { session: None }
        ) {
            return self.broadcast(request).await;
        }
        let request = self.route_archived(request).await?;
        for _ in 0..3 {
            let target = self.owner(&request).await?;
            let response = self.direct(target, request.clone(), None).await?;
            if let Response::Redirect { generation } = &response
                && redirect_allowed(&request)
            {
                self.verify_redirect(generation.clone()).await?;
                continue;
            }
            let response = response.checked()?;
            if let Some(generation) = archived_mutation(&request) {
                self.evict_historical(generation);
            }
            return Ok(response);
        }
        Err(IpcError::Unstable {
            request: request.name(),
        }
        .into())
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
