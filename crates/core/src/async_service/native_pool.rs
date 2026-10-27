use super::*;

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
            running: Arc::clone(&running),
        });
        for index in 0..count {
            let receiver = Arc::clone(&receiver);
            let running = Arc::clone(&running);
            owner
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(
                    std::thread::Builder::new()
                        .name(format!("{name}-{index}"))
                        .spawn(move || {
                            loop {
                                let Some(job) = receiver
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .blocking_recv()
                                else {
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
    #[must_use]
    pub fn occupancy(&self) -> usize {
        self.owner.running.load(Ordering::Acquire)
    }
    #[must_use]
    pub fn queued(&self) -> usize {
        self.sender
            .max_capacity()
            .saturating_sub(self.sender.capacity())
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
        let count = buffer.len().min(limit.saturating_sub(bytes.len()));
        let Some(window) = buffer.get_mut(..count) else {
            break;
        };
        match reader.read(window) {
            Ok(0) => break,
            Ok(n) => {
                if let Some(chunk) = window.get(..n) {
                    bytes.extend_from_slice(chunk);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(bytes)
}
