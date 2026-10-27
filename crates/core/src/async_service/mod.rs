//! GUI-owned async supervision and bounded adapters for synchronous libraries.
//! No filesystem or DNS blocking work runs on the async executor.
use futures_util::{FutureExt, future::BoxFuture};
use std::{
    collections::VecDeque,
    future::Future,
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
    fn from_error(error: anyhow::Error) -> Self {
        let message = format!("{error:#}");
        if error
            .downcast_ref::<crate::async_client::UncertainMutation>()
            .is_some()
        {
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
    future: BoxFuture<'static, anyhow::Result<T>>,
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
pub struct Handle<T>(Arc<Shared<T>>);
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
impl<T: Send + 'static> Handle<T> {
    pub fn submit(
        &self,
        mut context: OperationContext,
        cancellation: CancellationToken,
        future: impl Future<Output = anyhow::Result<T>> + Send + 'static,
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
                future: future.boxed(),
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
pub struct Supervisor<T> {
    pub handle: Handle<T>,
    results: mpsc::Receiver<Completion<T>>,
    thread: Option<JoinHandle<()>>,
}
impl<T: Send + 'static> Supervisor<T> {
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
                runtime.block_on(supervise(receiver, complete, owner, Arc::new(wake)));
            })?;
        Ok(Self {
            handle: Handle(shared),
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
impl<T> Drop for Supervisor<T> {
    fn drop(&mut self) {
        self.handle.0.closed.store(true, Ordering::Release);
        self.handle.0.stop.cancel();
        // The coordinator owns the runtime and waits for accepted mutations.
        // Keep the UI free to complete native window teardown.
    }
}
struct DeliverInput<'a, T> {
    context: OperationContext,
    result: Result<T, Failure>,
    shared: &'a Shared<T>,
    complete: &'a mpsc::Sender<Completion<T>>,
    wake: &'a Arc<dyn Fn() + Send + Sync>,
}

async fn deliver<T>(input: DeliverInput<'_, T>) {
    let DeliverInput {
        context,
        result,
        shared,
        complete,
        wake,
    } = input;
    shared.counters.completed.fetch_add(1, Ordering::Relaxed);
    if matches!(result, Err(Failure::Panicked)) {
        shared.counters.panics.fetch_add(1, Ordering::Relaxed);
    }
    let _ = complete
        .send(Completion {
            context,
            result: Some(result),
            counters: Arc::clone(&shared.counters),
        })
        .await;
    wake();
}
async fn supervise<T: Send + 'static>(
    mut requests: mpsc::Receiver<ServiceRequest<T>>,
    complete: mpsc::Sender<Completion<T>>,
    shared: Arc<Shared<T>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut tasks = JoinSet::new();
    let mut active = Vec::<(OperationContext, CancellationToken)>::new();
    let mut pending = VecDeque::<ServiceRequest<T>>::new();
    let mut stopping = false;
    let mut epoch = shared.read_epoch.load(Ordering::Acquire);
    loop {
        epoch = refresh_epoch(&shared, &active, epoch);
        store_oldest(&shared, &active, &pending);
        admit_pending(AdmitPendingInput {
            pending: &mut pending,
            active: &mut active,
            tasks: &mut tasks,
            shared: &shared,
            complete: &complete,
            wake: &wake,
            epoch,
            stopping,
        })
        .await;
        if stopping && tasks.is_empty() && pending.is_empty() && requests.is_empty() {
            break;
        }
        tokio::select! {
            () = shared.stop.cancelled(), if !stopping => {
                stopping = true;
                requests.close();
                for (context, cancellation) in &active {
                    if context.policy != Policy::OrderedMutation { cancellation.cancel(); }
                }
            }
            Some(job) = requests.recv() => {
                cancel_replaceable(&active, &pending, &job);
                pending.push_back(job);
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                // User future panics are caught inside the task, retaining its operation identity.
                if let Ok((context, mut result, read_epoch)) = result {
                    if context.policy == Policy::ReplaceableRead && read_epoch != shared.read_epoch.load(Ordering::Acquire) { result = Err(Failure::Cancelled); }
                    active.retain(|(c, _)| c.id != context.id);
                    if context.policy == Policy::ServiceLifetime { shared.counters.actors.fetch_sub(1, Ordering::AcqRel); }
                    else { shared.counters.active.fetch_sub(1, Ordering::AcqRel); }
                    deliver(DeliverInput { context, result, shared: &shared, complete: &complete, wake: &wake }).await;
                }
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

fn refresh_epoch<T>(
    shared: &Shared<T>,
    active: &[(OperationContext, CancellationToken)],
    epoch: u64,
) -> u64 {
    let current = shared.read_epoch.load(Ordering::Acquire);
    if epoch != current {
        for (context, token) in active {
            if context.policy == Policy::ReplaceableRead {
                token.cancel();
            }
        }
    }
    current
}

fn store_oldest<T>(
    shared: &Shared<T>,
    active: &[(OperationContext, CancellationToken)],
    pending: &VecDeque<ServiceRequest<T>>,
) {
    let age = active
        .iter()
        .map(|(c, _)| c)
        .chain(pending.iter().map(|r| &r.context))
        .filter(|c| c.policy != Policy::ServiceLifetime)
        .map(|c| u64::try_from(c.admitted.elapsed().as_millis()).unwrap_or(u64::MAX))
        .max()
        .unwrap_or(0);
    shared.counters.oldest_ms.store(age, Ordering::Release);
}

fn cancel_replaceable<T>(
    active: &[(OperationContext, CancellationToken)],
    pending: &VecDeque<ServiceRequest<T>>,
    job: &ServiceRequest<T>,
) {
    if job.context.policy != Policy::ReplaceableRead {
        return;
    }
    for (context, cancellation) in active {
        if context.subsystem == job.context.subsystem
            && context.resource == job.context.resource
            && context.policy == Policy::ReplaceableRead
        {
            cancellation.cancel();
        }
    }
    for previous in pending {
        if previous.context.subsystem == job.context.subsystem
            && previous.context.resource == job.context.resource
            && previous.context.policy == Policy::ReplaceableRead
        {
            previous.cancellation.cancel();
        }
    }
}

struct AdmitPendingInput<'a, T> {
    pending: &'a mut VecDeque<ServiceRequest<T>>,
    active: &'a mut Vec<(OperationContext, CancellationToken)>,
    tasks: &'a mut JoinSet<(OperationContext, Result<T, Failure>, u64)>,
    shared: &'a Shared<T>,
    complete: &'a mpsc::Sender<Completion<T>>,
    wake: &'a Arc<dyn Fn() + Send + Sync>,
    epoch: u64,
    stopping: bool,
}

async fn admit_pending<T: Send + 'static>(input: AdmitPendingInput<'_, T>) {
    let AdmitPendingInput {
        pending,
        active,
        tasks,
        shared,
        complete,
        wake,
        epoch,
        stopping,
    } = input;
    let mut index = 0;
    while index < pending.len() {
        let Some(job) = pending.get(index) else {
            break;
        };
        let cancelled = job.cancellation.is_cancelled()
            || (job.context.policy == Policy::ReplaceableRead && job.read_epoch != epoch)
            || (stopping && job.context.policy != Policy::OrderedMutation);
        let expired = job.context.deadline.is_some_and(|d| Instant::now() >= d);
        if cancelled || expired {
            let Some(job) = pending.remove(index) else {
                break;
            };
            shared.counters.queued.fetch_sub(1, Ordering::AcqRel);
            deliver(DeliverInput {
                context: job.context,
                result: Err(if cancelled {
                    Failure::Cancelled
                } else {
                    Failure::Timeout
                }),
                shared,
                complete,
                wake,
            })
            .await;
            continue;
        }
        let actor = job.context.policy == Policy::ServiceLifetime;
        let busy = active.iter().any(|(c, _)| {
            c.resource == job.context.resource && c.subsystem == job.context.subsystem
        });
        if busy
            || (!actor && shared.counters.active.load(Ordering::Acquire) >= ACTIVE_LIMIT)
            || (actor && shared.counters.actors.load(Ordering::Acquire) >= ACTOR_LIMIT)
        {
            index = index.saturating_add(1);
            continue;
        }
        let Some(job) = pending.remove(index) else {
            break;
        };
        shared.counters.queued.fetch_sub(1, Ordering::AcqRel);
        if actor {
            shared.counters.actors.fetch_add(1, Ordering::AcqRel);
        } else {
            shared.counters.active.fetch_add(1, Ordering::AcqRel);
        }
        active.push((job.context.clone(), job.cancellation.clone()));
        tasks.spawn(async move {
                let _cancel_native_reads = (job.context.policy == Policy::ReplaceableRead).then(|| job.cancellation.clone().drop_guard());
                let result = AssertUnwindSafe(async {
                    // Mutations own their deadlines: transport errors after write are uncertain,
                    // and native writes must remain tracked until the library returns.
                    if job.context.policy == Policy::ReplaceableRead {
                        let deadline = job.context.deadline;
                        tokio::select! {
                            biased;
                            () = job.cancellation.cancelled() => Err(Failure::Cancelled),
                            () = async { match deadline { Some(d) => tokio::time::sleep_until(d.into()).await, None => std::future::pending().await } } => Err(Failure::Timeout),
                            result = job.future => result.map_err(Failure::from_error),
                        }
                    } else {
                        job.future.await.map_err(Failure::from_error)
                    }
                }).catch_unwind().await.unwrap_or(Err(Failure::Panicked));
                (job.context, result, job.read_epoch)
            });
    }
}

mod native_pool;
pub use native_pool::{NativePool, read_chunks};

#[cfg(test)]
mod tests;
