use super::connection::{ConnectionSpec, Failure};
use super::*;

// Only closing the input channel cancels a retry. Keys from the old connection
// must be discarded without restarting the delay or ending the worker.
async fn wait_retry_tick(input_rx: &mut mpsc::Receiver<VncInputEvent>) -> bool {
    let delay = tokio::time::sleep(Duration::from_secs(1));
    tokio::pin!(delay);
    loop {
        tokio::select! {
            () = &mut delay => return true,
            event = input_rx.recv() => if event.is_none() { return false; },
        }
    }
}

pub(super) struct RunInput {
    pub shared: Arc<Shared>,
    pub runtime: Arc<Mutex<VncRuntime>>,
    pub sid: String,
    pub spec: ConnectionSpec,
    pub input_rx: mpsc::Receiver<VncInputEvent>,
}
pub(super) async fn run(input: RunInput) {
    let RunInput {
        shared,
        runtime,
        sid,
        spec,
        mut input_rx,
    } = input;
    let mut attempt = 0u8;
    loop {
        // Never replay keys queued for a previous connection.
        while input_rx.try_recv().is_ok() {}
        let started = Instant::now();
        let result = super::connection::run(super::connection::RunInput {
            shared: &shared,
            runtime: &runtime,
            sid: &sid,
            spec: &spec,
            input_rx: &mut input_rx,
        })
        .await;
        match result {
            Err(Failure::Cancelled) | Ok(()) => {
                break;
            }
            Err(Failure::Permanent(message)) => {
                mark_failed(MarkFailedInput {
                    shared: &shared,
                    runtime: &runtime,
                    session_id: &sid,
                    message,
                });
                break;
            }
            Err(Failure::Network(message)) => {
                if started.elapsed() >= Duration::from_mins(1) {
                    attempt = 0;
                }
                if !spec.options.reconnect.enabled || attempt >= spec.options.reconnect.max_attempts
                {
                    mark_disconnected(MarkDisconnectedInput {
                        shared: &shared,
                        runtime: &runtime,
                        session_id: &sid,
                        reason: Some(message),
                    });
                    break;
                }
                let delay = spec.options.reconnect.delay_seconds(attempt);
                attempt = attempt.saturating_add(1);
                {
                    let mut state = relock(&shared.state);
                    if let Some(session) = state.vnc_sessions.iter_mut().find(|s| s.id == sid) {
                        session.status = VncSessionStatus::Connecting;
                        session.error = Some(message.clone());
                        session.retry_attempt = attempt;
                        session.retry_in_seconds = delay;
                    }
                    state.revision = state.revision.saturating_add(1);
                }
                {
                    let mut rt = relock(&runtime);
                    rt.status = VncSessionStatus::Connecting;
                    rt.error = Some(format!("Retry {attempt} in {delay}s: {message}"));
                    rt.revision = rt.revision.saturating_add(1);
                }
                let mut cancelled = false;
                for remaining in (0..delay).rev() {
                    if !wait_retry_tick(&mut input_rx).await {
                        cancelled = true;
                        break;
                    }
                    let mut state = relock(&shared.state);
                    if let Some(session) = state.vnc_sessions.iter_mut().find(|s| s.id == sid) {
                        session.retry_in_seconds = remaining;
                    }
                    state.revision = state.revision.saturating_add(1);
                }
                if cancelled {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_keys_do_not_cancel_retry_but_disconnect_does() {
        let (tx, mut rx) = mpsc::channel(4);
        tx.send(VncInputEvent::Key {
            keycode: 65,
            down: true,
        })
        .await
        .unwrap();
        tx.send(VncInputEvent::Refresh { full: true })
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), wait_retry_tick(&mut rx))
                .await
                .unwrap()
        );
        assert!(rx.try_recv().is_err());
        drop(tx);
        assert!(
            !tokio::time::timeout(Duration::from_millis(100), wait_retry_tick(&mut rx))
                .await
                .unwrap()
        );
    }
}
