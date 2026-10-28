//! GUI-owned async supervision and bounded adapters for synchronous libraries.
//! No filesystem or DNS blocking work runs on the async executor.
use futures_util::{FutureExt, future::BoxFuture};
use std::{
    collections::VecDeque,
    future::Future,
    marker::PhantomData,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};
pub use tokio_util::sync::CancellationToken;

pub const ACTIVE_LIMIT: usize = 32;
/// Concurrent `ServiceLifetime` actors. App-wide services take about eight,
/// and every language server for a (language, root) pair is another one, so
/// a monorepo with many package roots needs far more than a handful.
pub const ACTOR_LIMIT: usize = 64;
pub const QUEUE_LIMIT: usize = 128;
pub const NATIVE_QUEUE_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OperationId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    ReplaceableRead,
    OrderedMutation,
    ServiceLifetime,
}
#[derive(Clone, Debug)]
pub struct OperationContext {
    pub id: OperationId,
    pub subsystem: &'static str,
    pub project: Option<String>,
    pub tab: Option<String>,
    pub session: Option<String>,
    pub resource: String,
    pub generation: u64,
    pub deadline: Option<Instant>,
    pub policy: Policy,
    pub admitted: Instant,
}
impl OperationContext {
    #[must_use]
    pub fn new(subsystem: &'static str, resource: String, policy: Policy) -> Self {
        Self {
            id: OperationId(0),
            subsystem,
            project: None,
            tab: None,
            session: None,
            resource,
            generation: 0,
            deadline: Some(
                Instant::now()
                    .checked_add(Duration::from_secs(30))
                    .unwrap_or_else(Instant::now),
            ),
            policy,
            admitted: Instant::now(),
        }
    }
}
#[derive(Debug)]
pub enum Failure {
    Cancelled,
    Timeout,
    Overloaded,
    Closed,
    Panicked,
    Failed(String),
    UncertainMutation(String),
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Operation cancelled"),
            Self::Timeout => f.write_str("Operation timed out"),
            Self::Overloaded => f.write_str("Services are busy; retry the action"),
            Self::Closed => f.write_str("Services are closing"),
            Self::Panicked => f.write_str("Service task panicked"),
            Self::Failed(message) | Self::UncertainMutation(message) => f.write_str(message),
        }
    }
}
impl Failure {
    /// Keeps the full source chain as text. A failure anywhere in the chain
    /// that left a mutation's outcome unknown makes this `UncertainMutation`.
    fn from_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        let error: Box<dyn std::error::Error + Send + Sync> = error.into();
        let root: &(dyn std::error::Error + 'static) = &*error;
        let message = crate::ErrorChain(root).to_string();
        let uncertain = std::iter::successors(Some(root), |e| e.source()).any(|e| {
            e.downcast_ref::<crate::async_client::AsyncClientError>()
                .is_some_and(crate::async_client::AsyncClientError::is_uncertain_mutation)
        });
        if uncertain {
            Self::UncertainMutation(message)
        } else {
            Self::Failed(message)
        }
    }
}
impl std::error::Error for Failure {}
#[derive(Default)]
struct Counters {
    required: AtomicUsize,
    outstanding: AtomicUsize,
    queued: AtomicUsize,
    active: AtomicUsize,
    actors: AtomicUsize,
    completed: AtomicU64,
    panics: AtomicU64,
    oldest_ms: AtomicU64,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct Diagnostics {
    pub required: usize,
    pub outstanding: usize,
    pub queued: usize,
    pub active: usize,
    pub actors: usize,
    pub completed: u64,
    pub panics: u64,
    pub oldest_ms: u64,
}

pub struct Completion<T> {
    pub context: OperationContext,
    pub result: Option<Result<T, Failure>>,
    counters: Arc<Counters>,
}
impl<T> Drop for Completion<T> {
    fn drop(&mut self) {
        self.counters.outstanding.fetch_sub(1, Ordering::AcqRel);
        if self.context.policy == Policy::OrderedMutation {
            self.counters.required.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
struct ServiceRequest<T> {
    context: OperationContext,
    cancellation: CancellationToken,
    future: BoxFuture<'static, Result<T, Failure>>,
    read_epoch: u64,
}
struct Shared<T> {
    requests: mpsc::Sender<ServiceRequest<T>>,
    closed: AtomicBool,
    stop: CancellationToken,
    next: AtomicU64,
    read_epoch: AtomicU64,
    counters: Arc<Counters>,
}
/// Submits work whose futures fail with `E`. A failure reaches the completion
/// as [`Failure::Failed`] (or [`Failure::UncertainMutation`]) with the text of
/// `E` and its sources.
pub struct Handle<T, E>(Arc<Shared<T>>, PhantomData<fn() -> E>);
impl<T, E> Clone for Handle<T, E> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0), PhantomData)
    }
}
impl<T: Send + 'static, E: Into<Box<dyn std::error::Error + Send + Sync>> + 'static> Handle<T, E> {
    pub fn submit(
        &self,
        mut context: OperationContext,
        cancellation: CancellationToken,
        future: impl Future<Output = Result<T, E>> + Send + 'static,
    ) -> Result<OperationId, Failure> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(Failure::Closed);
        }
        if cancellation.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        self.0
            .counters
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < QUEUE_LIMIT).then_some(n.saturating_add(1))
            })
            .map_err(|_| Failure::Overloaded)?;
        context.id = OperationId(self.0.next.fetch_add(1, Ordering::Relaxed));
        let id = context.id;
        let required = context.policy == Policy::OrderedMutation;
        if required {
            self.0.counters.required.fetch_add(1, Ordering::AcqRel);
        }
        self.0.counters.outstanding.fetch_add(1, Ordering::AcqRel);
        if self
            .0
            .requests
            .try_send(ServiceRequest {
                context,
                cancellation,
                read_epoch: self.0.read_epoch.load(Ordering::Acquire),
                future: future
                    .map(|result| result.map_err(Failure::from_error))
                    .boxed(),
            })
            .is_err()
        {
            if required {
                self.0.counters.required.fetch_sub(1, Ordering::AcqRel);
            }
            self.0.counters.queued.fetch_sub(1, Ordering::AcqRel);
            self.0.counters.outstanding.fetch_sub(1, Ordering::AcqRel);
            return Err(Failure::Overloaded);
        }
        Ok(id)
    }
    #[must_use]
    pub fn diagnostics(&self) -> Diagnostics {
        let c = &self.0.counters;
        Diagnostics {
            required: c.required.load(Ordering::Acquire),
            outstanding: c.outstanding.load(Ordering::Acquire),
            queued: c.queued.load(Ordering::Acquire),
            active: c.active.load(Ordering::Acquire),
            actors: c.actors.load(Ordering::Acquire),
            completed: c.completed.load(Ordering::Acquire),
            panics: c.panics.load(Ordering::Acquire),
            oldest_ms: c.oldest_ms.load(Ordering::Acquire),
        }
    }
    pub fn cancel_reads(&self) {
        self.0.read_epoch.fetch_add(1, Ordering::AcqRel);
    }
    pub fn close_admission(&self) {
        self.0.closed.store(true, Ordering::Release);
    }
    pub fn reopen_admission(&self) {
        if !self.0.stop.is_cancelled() {
            self.0.closed.store(false, Ordering::Release);
        }
    }
}
/// Runtime destruction happens on the coordinator thread, never in a UI callback.
pub struct Supervisor<T, E> {
    pub handle: Handle<T, E>,
    results: mpsc::Receiver<Completion<T>>,
    thread: Option<JoinHandle<()>>,
}
impl<T: Send + 'static, E> Supervisor<T, E> {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let (requests, receiver) = mpsc::channel(QUEUE_LIMIT);
        let (complete, results) = mpsc::channel(QUEUE_LIMIT);
        let shared = Arc::new(Shared {
            requests,
            closed: AtomicBool::new(false),
            stop: CancellationToken::new(),
            next: AtomicU64::new(1),
            read_epoch: AtomicU64::new(0),
            counters: Arc::default(),
        });
        let owner = Arc::clone(&shared);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("gui-async")
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("gui-supervisor".into())
            .spawn(move || {
                runtime.block_on(supervise(SuperviseInput {
                    requests: receiver,
                    complete,
                    shared: owner,
                    wake: Arc::new(wake),
                }));
            })?;
        Ok(Self {
            handle: Handle(shared, PhantomData),
            results,
            thread: Some(thread),
        })
    }
    pub fn try_recv(&mut self) -> Option<Completion<T>> {
        self.results.try_recv().ok()
    }
    #[must_use]
    pub fn has_results(&self) -> bool {
        !self.results.is_empty()
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl<T, E> Drop for Supervisor<T, E> {
    fn drop(&mut self) {
        self.handle.0.closed.store(true, Ordering::Release);
        self.handle.0.stop.cancel();
        // The coordinator owns the runtime and waits for accepted mutations.
        // Keep the UI free to complete native window teardown.
    }
}
mod native_pool;
mod supervise;
pub use native_pool::{ChunkReadError, NativePool, read_chunks};
use supervise::{SuperviseInput, supervise};

#[cfg(test)]
mod tests;
