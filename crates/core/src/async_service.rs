//! GUI-owned async supervision and bounded adapters for synchronous libraries.
//! No filesystem or DNS blocking work runs on the async executor.
use futures_util::{FutureExt, future::BoxFuture};
use std::{
    collections::VecDeque,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
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
    pub fn new(subsystem: &'static str, resource: String, policy: Policy) -> Self {
        Self {
            id: OperationId(0),
            subsystem,
            project: None,
            tab: None,
            session: None,
            resource,
            generation: 0,
            deadline: Some(Instant::now() + Duration::from_secs(30)),
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
        Self(self.0.clone())
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
                (n < QUEUE_LIMIT).then_some(n + 1)
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
        let owner = shared.clone();
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
async fn deliver<T>(
    context: OperationContext,
    result: Result<T, Failure>,
    shared: &Shared<T>,
    complete: &mpsc::Sender<Completion<T>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    shared.counters.completed.fetch_add(1, Ordering::Relaxed);
    if matches!(result, Err(Failure::Panicked)) {
        shared.counters.panics.fetch_add(1, Ordering::Relaxed);
    }
    let _ = complete
        .send(Completion {
            context,
            result: Some(result),
            counters: shared.counters.clone(),
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
        let current = shared.read_epoch.load(Ordering::Acquire);
        if epoch != current {
            epoch = current;
            for (context, token) in &active {
                if context.policy == Policy::ReplaceableRead {
                    token.cancel();
                }
            }
        }
        let age = active
            .iter()
            .map(|(c, _)| c)
            .chain(pending.iter().map(|r| &r.context))
            .filter(|c| c.policy != Policy::ServiceLifetime)
            .map(|c| c.admitted.elapsed().as_millis() as u64)
            .max()
            .unwrap_or(0);
        shared.counters.oldest_ms.store(age, Ordering::Release);

        let mut index = 0;
        while index < pending.len() {
            let job = &pending[index];
            let cancelled = job.cancellation.is_cancelled()
                || (job.context.policy == Policy::ReplaceableRead && job.read_epoch != epoch)
                || (stopping && job.context.policy != Policy::OrderedMutation);
            let expired = job.context.deadline.is_some_and(|d| Instant::now() >= d);
            if cancelled || expired {
                let job = pending.remove(index).unwrap();
                shared.counters.queued.fetch_sub(1, Ordering::AcqRel);
                deliver(
                    job.context,
                    Err(if cancelled {
                        Failure::Cancelled
                    } else {
                        Failure::Timeout
                    }),
                    &shared,
                    &complete,
                    &wake,
                )
                .await;
                continue;
            }
            let actor = job.context.policy == Policy::ServiceLifetime;
            let busy = active.iter().any(|(c, _)| {
                c.resource == job.context.resource && c.subsystem == job.context.subsystem
            });
            if busy
                || (!actor && shared.counters.active.load(Ordering::Acquire) >= ACTIVE_LIMIT)
                || (actor && shared.counters.actors.load(Ordering::Acquire) >= 16)
            {
                index += 1;
                continue;
            }
            let job = pending.remove(index).unwrap();
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
                    if job.context.policy != Policy::ReplaceableRead {
                        job.future.await.map_err(Failure::from_error)
                    } else {
                        let deadline = job.context.deadline;
                        tokio::select! {
                            biased;
                            _ = job.cancellation.cancelled() => Err(Failure::Cancelled),
                            _ = async { match deadline { Some(d) => tokio::time::sleep_until(d.into()).await, None => std::future::pending().await } } => Err(Failure::Timeout),
                            result = job.future => result.map_err(Failure::from_error),
                        }
                    }
                }).catch_unwind().await.unwrap_or(Err(Failure::Panicked));
                (job.context, result, job.read_epoch)
            });
        }
        if stopping && tasks.is_empty() && pending.is_empty() && requests.is_empty() {
            break;
        }
        tokio::select! {
            _ = shared.stop.cancelled(), if !stopping => {
                stopping = true;
                requests.close();
                for (context, cancellation) in &active {
                    if context.policy != Policy::OrderedMutation { cancellation.cancel(); }
                }
            }
            Some(job) = requests.recv() => {
                if job.context.policy == Policy::ReplaceableRead {
                    for (context, cancellation) in &active {
                        if context.subsystem == job.context.subsystem && context.resource == job.context.resource
                            && context.policy == Policy::ReplaceableRead { cancellation.cancel(); }
                    }
                    for previous in &pending {
                        if previous.context.subsystem == job.context.subsystem && previous.context.resource == job.context.resource
                            && previous.context.policy == Policy::ReplaceableRead { previous.cancellation.cancel(); }
                    }
                }
                pending.push_back(job);
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                // User future panics are caught inside the task, retaining its operation identity.
                if let Ok((context, mut result, read_epoch)) = result {
                    if context.policy == Policy::ReplaceableRead && read_epoch != shared.read_epoch.load(Ordering::Acquire) { result = Err(Failure::Cancelled); }
                    active.retain(|(c, _)| c.id != context.id);
                    if context.policy == Policy::ServiceLifetime { shared.counters.actors.fetch_sub(1, Ordering::AcqRel); }
                    else { shared.counters.active.fetch_sub(1, Ordering::AcqRel); }
                    deliver(context, result, &shared, &complete, &wake).await;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

type NativeCall = Box<dyn FnOnce() + Send>;
struct NativeJob {
    call: NativeCall,
    cancellation: CancellationToken,
}
struct PoolOwner {
    threads: Mutex<Vec<JoinHandle<()>>>,
    running: Arc<AtomicUsize>,
}
#[derive(Clone)]
pub struct NativePool {
    sender: mpsc::Sender<NativeJob>,
    owner: Arc<PoolOwner>,
}
impl NativePool {
    pub fn new(name: &str, count: usize) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<NativeJob>(NATIVE_QUEUE_LIMIT);
        let receiver = Arc::new(Mutex::new(receiver));
        let running = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(PoolOwner {
            threads: Mutex::new(Vec::new()),
            running: running.clone(),
        });
        for index in 0..count {
            let receiver = receiver.clone();
            let running = running.clone();
            owner.threads.lock().unwrap().push(
                std::thread::Builder::new()
                    .name(format!("{name}-{index}"))
                    .spawn(move || {
                        loop {
                            let Some(job) = receiver.lock().unwrap().blocking_recv() else {
                                break;
                            };
                            if job.cancellation.is_cancelled() {
                                continue;
                            }
                            running.fetch_add(1, Ordering::AcqRel);
                            let _ = std::panic::catch_unwind(AssertUnwindSafe(job.call));
                            running.fetch_sub(1, Ordering::AcqRel);
                        }
                    })?,
            );
        }
        Ok(Self { sender, owner })
    }
    pub async fn run<T: Send + 'static>(
        &self,
        cancellation: &CancellationToken,
        call: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        if cancellation.is_cancelled() {
            return Err(Failure::Cancelled.into());
        }
        let cancellation = cancellation.child_token();
        let _cancel_queued_on_drop = cancellation.clone().drop_guard();
        let (reply, response) = oneshot::channel();
        let job = NativeJob {
            cancellation: cancellation.clone(),
            call: Box::new(move || {
                let result = std::panic::catch_unwind(AssertUnwindSafe(call))
                    .unwrap_or_else(|_| Err(Failure::Panicked.into()));
                let _ = reply.send(result);
            }),
        };
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(Failure::Cancelled.into()),
            result = self.sender.send(job) => result.map_err(|_| Failure::Closed)?,
        }
        response.await.map_err(|_| {
            if cancellation.is_cancelled() {
                Failure::Cancelled
            } else {
                Failure::Closed
            }
        })?
    }
    pub fn occupancy(&self) -> usize {
        self.owner.running.load(Ordering::Acquire)
    }
    pub fn queued(&self) -> usize {
        self.sender.max_capacity() - self.sender.capacity()
    }
}

/// Bounded cooperative reads for native filesystem adapters. A single OS read
/// may still block; cancellation is checked before the next controllable chunk.
pub fn read_chunks(
    mut reader: impl std::io::Read,
    limit: usize,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    while bytes.len() < limit {
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled.into());
        }
        let count = buffer.len().min(limit - bytes.len());
        match reader.read(&mut buffer[..count]) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn completion<T: Send + 'static>(supervisor: &mut Supervisor<T>) -> Completion<T> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = supervisor.try_recv() {
                return result;
            }
            assert!(Instant::now() < deadline, "supervisor did not complete");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    fn context(key: &str, policy: Policy) -> OperationContext {
        OperationContext::new("test", key.into(), policy)
    }
    #[test]
    fn cancellation_between_file_chunks_stops_before_another_read() {
        struct Reader {
            token: CancellationToken,
            reads: Arc<AtomicUsize>,
        }
        impl std::io::Read for Reader {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                self.reads.fetch_add(1, Ordering::AcqRel);
                bytes[0] = 1;
                self.token.cancel();
                Ok(1)
            }
        }
        let token = CancellationToken::new();
        let reads = Arc::new(AtomicUsize::new(0));
        assert!(
            read_chunks(
                Reader {
                    token: token.clone(),
                    reads: reads.clone()
                },
                2,
                &token
            )
            .is_err()
        );
        assert_eq!(reads.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn panics_are_reaped_and_required_barrier_includes_unapplied_results() {
        let mut supervisor = Supervisor::<usize>::new(|| {}).unwrap();
        supervisor
            .handle
            .submit(
                context("write", Policy::OrderedMutation),
                CancellationToken::new(),
                async { Ok(42) },
            )
            .unwrap();
        let result = completion(&mut supervisor).await;
        assert_eq!(supervisor.handle.diagnostics().required, 1);
        assert!(matches!(result.result, Some(Ok(42))));
        drop(result);
        assert_eq!(supervisor.handle.diagnostics().required, 0);
        supervisor
            .handle
            .submit(
                context("panic", Policy::ReplaceableRead),
                CancellationToken::new(),
                async {
                    panic!("fixture panic");
                    #[allow(unreachable_code)]
                    Ok(0)
                },
            )
            .unwrap();
        let result = completion(&mut supervisor).await;
        assert!(matches!(result.result, Some(Err(Failure::Panicked))));
        drop(result);
        let diagnostics = supervisor.handle.diagnostics();
        assert_eq!(diagnostics.active, 0);
        assert_eq!(diagnostics.outstanding, 0);
        assert_eq!(diagnostics.panics, 1);
    }
    #[tokio::test]
    async fn cancellation_coalesces_reads_and_independent_keys_progress() {
        let mut supervisor = Supervisor::<usize>::new(|| {}).unwrap();
        let token = CancellationToken::new();
        supervisor
            .handle
            .submit(
                context("stalled", Policy::ReplaceableRead),
                token.clone(),
                async { std::future::pending().await },
            )
            .unwrap();
        supervisor
            .handle
            .submit(
                context("other", Policy::OrderedMutation),
                CancellationToken::new(),
                async { Ok(2) },
            )
            .unwrap();
        assert!(matches!(
            completion(&mut supervisor).await.result,
            Some(Ok(2))
        ));
        supervisor
            .handle
            .submit(
                context("stalled", Policy::ReplaceableRead),
                CancellationToken::new(),
                async { Ok(3) },
            )
            .unwrap();
        let first = completion(&mut supervisor).await;
        assert!(matches!(first.result, Some(Err(Failure::Cancelled))));
        drop(first);
        assert!(token.is_cancelled());
        assert!(matches!(
            completion(&mut supervisor).await.result,
            Some(Ok(3))
        ));
        assert_eq!(supervisor.handle.diagnostics().outstanding, 0);
    }
    #[tokio::test]
    async fn admission_overflow_is_explicit_and_cancelled_actions_never_start() {
        let mut supervisor = Supervisor::<usize>::new(|| {}).unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            supervisor.handle.submit(
                context("unused", Policy::OrderedMutation),
                cancelled,
                async { panic!("must not run") }
            ),
            Err(Failure::Cancelled)
        ));
        let release = Arc::new(tokio::sync::Notify::new());
        let gate = release.clone();
        supervisor
            .handle
            .submit(
                context("ordered", Policy::OrderedMutation),
                CancellationToken::new(),
                async move {
                    gate.notified().await;
                    Ok(0)
                },
            )
            .unwrap();
        let mut accepted = 1;
        for _ in 0..(QUEUE_LIMIT + ACTIVE_LIMIT + 1) {
            match supervisor.handle.submit(
                context("ordered", Policy::OrderedMutation),
                CancellationToken::new(),
                async { Ok(1) },
            ) {
                Ok(_) => accepted += 1,
                Err(Failure::Overloaded) => break,
                Err(error) => panic!("{error}"),
            }
        }
        assert!(accepted <= QUEUE_LIMIT + 1);
        supervisor.handle.close_admission();
        assert!(matches!(
            supervisor.handle.submit(
                context("closed", Policy::OrderedMutation),
                CancellationToken::new(),
                async { Ok(0) }
            ),
            Err(Failure::Closed)
        ));
        release.notify_one();
        for _ in 0..accepted {
            drop(completion(&mut supervisor).await);
        }
        assert_eq!(supervisor.handle.diagnostics().required, 0);
        assert_eq!(supervisor.handle.diagnostics().queued, 0);
        assert_eq!(supervisor.handle.diagnostics().active, 0);
    }
    #[tokio::test]
    async fn native_cancel_does_not_replace_a_blocked_worker() {
        let pool = NativePool::new("native-test", 1).unwrap();
        let (entered, entry) = oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let token = CancellationToken::new();
        let task_pool = pool.clone();
        let task_token = token.clone();
        let active = tokio::spawn(async move {
            task_pool
                .run(&task_token, move || {
                    let _ = entered.send(());
                    gate.recv().unwrap();
                    Ok(7)
                })
                .await
        });
        entry.await.unwrap();
        token.cancel();
        assert_eq!(pool.occupancy(), 1);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            pool.run(&cancelled, || -> anyhow::Result<()> {
                panic!("cancelled native job ran")
            })
            .await
            .is_err()
        );
        assert!(!active.is_finished());
        release.send(()).unwrap();
        assert_eq!(active.await.unwrap().unwrap(), 7);
    }
    #[tokio::test]
    async fn native_pool_waits_for_a_slot_instead_of_shedding() {
        let pool = NativePool::new("native-wait", 1).unwrap();
        let (entered, entry) = oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let token = CancellationToken::new();
        let blocked = pool.clone();
        let blocked_token = token.clone();
        let active = tokio::spawn(async move {
            blocked
                .run(&blocked_token, move || {
                    let _ = entered.send(());
                    gate.recv().unwrap();
                    Ok(1_u8)
                })
                .await
        });
        entry.await.unwrap();
        let mut extra = Vec::new();
        for n in 0..(NATIVE_QUEUE_LIMIT + 4) {
            let waiter = pool.clone();
            extra.push(tokio::spawn(async move {
                waiter
                    .run(&CancellationToken::new(), move || Ok(n as u8))
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            extra.iter().any(|task| !task.is_finished()),
            "waiters should block until the worker is free"
        );
        release.send(()).unwrap();
        assert_eq!(active.await.unwrap().unwrap(), 1);
        for (n, task) in extra.into_iter().enumerate() {
            assert_eq!(task.await.unwrap().unwrap(), n as u8);
        }
    }
    #[tokio::test]
    async fn continuous_completion_reaping_does_not_retain_tasks() {
        let mut supervisor = Supervisor::new(|| {}).unwrap();
        for n in 0..256 {
            supervisor
                .handle
                .submit(
                    context("read", Policy::ReplaceableRead),
                    CancellationToken::new(),
                    async move { Ok(n) },
                )
                .unwrap();
            drop(completion(&mut supervisor).await);
        }
        let diagnostics = supervisor.handle.diagnostics();
        assert_eq!(diagnostics.completed, 256);
        assert_eq!(
            diagnostics.active + diagnostics.queued + diagnostics.outstanding,
            0
        );
    }
}
