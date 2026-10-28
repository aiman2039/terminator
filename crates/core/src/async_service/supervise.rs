//! The coordinator loop: admission, per-resource exclusivity, and completion delivery.
use super::*;

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
pub(super) struct SuperviseInput<T> {
    pub(super) requests: mpsc::Receiver<ServiceRequest<T>>,
    pub(super) complete: mpsc::Sender<Completion<T>>,
    pub(super) shared: Arc<Shared<T>>,
    pub(super) wake: Arc<dyn Fn() + Send + Sync>,
}

pub(super) async fn supervise<T: Send + 'static>(input: SuperviseInput<T>) {
    let SuperviseInput {
        mut requests,
        complete,
        shared,
        wake,
    } = input;
    let mut tasks = JoinSet::new();
    let mut active = Vec::<(OperationContext, CancellationToken)>::new();
    let mut pending = VecDeque::<ServiceRequest<T>>::new();
    let mut stopping = false;
    let mut epoch = shared.read_epoch.load(Ordering::Acquire);
    loop {
        epoch = refresh_epoch(&shared, &active, epoch);
        store_oldest(&shared, &active, &pending);
        Admission {
            pending: &mut pending,
            active: &mut active,
            tasks: &mut tasks,
            shared: &shared,
            complete: &complete,
            wake: &wake,
            epoch,
            stopping,
        }
        .run()
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

/// One admission pass over the queue: queued jobs that were cancelled or
/// expired complete at once, and jobs with a free slot and an idle resource start.
struct Admission<'a, T> {
    pending: &'a mut VecDeque<ServiceRequest<T>>,
    active: &'a mut Vec<(OperationContext, CancellationToken)>,
    tasks: &'a mut JoinSet<(OperationContext, Result<T, Failure>, u64)>,
    shared: &'a Shared<T>,
    complete: &'a mpsc::Sender<Completion<T>>,
    wake: &'a Arc<dyn Fn() + Send + Sync>,
    epoch: u64,
    stopping: bool,
}

impl<T: Send + 'static> Admission<'_, T> {
    async fn run(mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            let Some(job) = self.pending.get(index) else {
                break;
            };
            if let Some(failure) = dropped(job, self.epoch, self.stopping) {
                let Some(job) = self.pending.remove(index) else {
                    break;
                };
                self.shared.counters.queued.fetch_sub(1, Ordering::AcqRel);
                deliver(DeliverInput {
                    context: job.context,
                    result: Err(failure),
                    shared: self.shared,
                    complete: self.complete,
                    wake: self.wake,
                })
                .await;
                continue;
            }
            if !admissible(job, self.active, self.shared) {
                index = index.saturating_add(1);
                continue;
            }
            let Some(job) = self.pending.remove(index) else {
                break;
            };
            self.start(job);
        }
    }

    fn start(&mut self, job: ServiceRequest<T>) {
        let counters = &self.shared.counters;
        counters.queued.fetch_sub(1, Ordering::AcqRel);
        if job.context.policy == Policy::ServiceLifetime {
            counters.actors.fetch_add(1, Ordering::AcqRel);
        } else {
            counters.active.fetch_add(1, Ordering::AcqRel);
        }
        self.active
            .push((job.context.clone(), job.cancellation.clone()));
        self.tasks.spawn(run_job(job));
    }
}

/// The failure a queued job completes with instead of running, if any.
fn dropped<T>(job: &ServiceRequest<T>, epoch: u64, stopping: bool) -> Option<Failure> {
    let cancelled = job.cancellation.is_cancelled()
        || (job.context.policy == Policy::ReplaceableRead && job.read_epoch != epoch)
        || (stopping && job.context.policy != Policy::OrderedMutation);
    let expired = job.context.deadline.is_some_and(|d| Instant::now() >= d);
    if cancelled {
        Some(Failure::Cancelled)
    } else if expired {
        Some(Failure::Timeout)
    } else {
        None
    }
}

/// Whether `job`'s resource is idle and its kind of slot has room.
fn admissible<T>(
    job: &ServiceRequest<T>,
    active: &[(OperationContext, CancellationToken)],
    shared: &Shared<T>,
) -> bool {
    let actor = job.context.policy == Policy::ServiceLifetime;
    let busy = active
        .iter()
        .any(|(c, _)| c.resource == job.context.resource && c.subsystem == job.context.subsystem);
    !(busy
        || (!actor && shared.counters.active.load(Ordering::Acquire) >= ACTIVE_LIMIT)
        || (actor && shared.counters.actors.load(Ordering::Acquire) >= ACTOR_LIMIT))
}

async fn run_job<T>(job: ServiceRequest<T>) -> (OperationContext, Result<T, Failure>, u64) {
    let _cancel_native_reads = (job.context.policy == Policy::ReplaceableRead)
        .then(|| job.cancellation.clone().drop_guard());
    let result = AssertUnwindSafe(async {
        // Mutations own their deadlines: transport errors after write are uncertain,
        // and native writes must remain tracked until the library returns.
        if job.context.policy == Policy::ReplaceableRead {
            let deadline = job.context.deadline;
            tokio::select! {
                biased;
                () = job.cancellation.cancelled() => Err(Failure::Cancelled),
                () = async { match deadline { Some(d) => tokio::time::sleep_until(d.into()).await, None => std::future::pending().await } } => Err(Failure::Timeout),
                result = job.future => result,
            }
        } else {
            job.future.await
        }
    })
    .catch_unwind()
    .await
    .unwrap_or(Err(Failure::Panicked));
    (job.context, result, job.read_epoch)
}
