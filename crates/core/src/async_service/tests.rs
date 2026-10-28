use super::*;
/// The fixtures fail with `Failure` itself.
type Supervisor<T> = super::Supervisor<T, Failure>;
async fn completion<T: Send + 'static>(supervisor: &mut Supervisor<T>) -> Completion<T> {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(3))
        .unwrap_or_else(Instant::now);
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
                reads: Arc::clone(&reads)
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
                #[expect(unreachable_code, reason = "Keeps the block's Result type")]
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
async fn many_service_actors_run_at_once() {
    let mut supervisor = Supervisor::<usize>::new(|| {}).unwrap();
    let release = CancellationToken::new();
    let count = 24;
    for n in 0..count {
        let gate = release.clone();
        supervisor
            .handle
            .submit(
                context(&format!("actor-{n}"), Policy::ServiceLifetime),
                CancellationToken::new(),
                async move {
                    gate.cancelled().await;
                    Ok(n)
                },
            )
            .unwrap();
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(3))
        .unwrap_or_else(Instant::now);
    while supervisor.handle.diagnostics().actors < count {
        assert!(Instant::now() < deadline, "actors were not all admitted");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    release.cancel();
    for _ in 0..count {
        drop(completion(&mut supervisor).await);
    }
    assert_eq!(supervisor.handle.diagnostics().actors, 0);
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
    let gate = Arc::clone(&release);
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
    let mut accepted: usize = 1;
    for _ in 0..QUEUE_LIMIT.saturating_add(ACTIVE_LIMIT).saturating_add(1) {
        match supervisor.handle.submit(
            context("ordered", Policy::OrderedMutation),
            CancellationToken::new(),
            async { Ok(1) },
        ) {
            Ok(_) => accepted = accepted.saturating_add(1),
            Err(Failure::Overloaded) => break,
            Err(error) => panic!("{error}"),
        }
    }
    assert!(accepted <= QUEUE_LIMIT.saturating_add(1));
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
                Ok::<_, Failure>(7)
            })
            .await
    });
    entry.await.unwrap();
    token.cancel();
    assert_eq!(pool.occupancy(), 1);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        pool.run(&cancelled, || -> Result<(), Failure> {
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
                Ok::<_, Failure>(1_u8)
            })
            .await
    });
    entry.await.unwrap();
    let mut extra = Vec::new();
    for n in 0..(NATIVE_QUEUE_LIMIT + 4) {
        let waiter = pool.clone();
        extra.push(tokio::spawn(async move {
            waiter
                .run(&CancellationToken::new(), move || {
                    Ok::<_, Failure>(u8::try_from(n).unwrap_or(u8::MAX))
                })
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
        assert_eq!(
            task.await.unwrap().unwrap(),
            u8::try_from(n).unwrap_or(u8::MAX)
        );
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
