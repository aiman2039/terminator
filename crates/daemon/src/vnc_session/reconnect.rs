use super::connection::{ConnectionSpec, Failure};
use super::*;

pub(super) async fn run(
    shared: Arc<Shared>,
    runtime: Arc<Mutex<VncRuntime>>,
    sid: String,
    spec: ConnectionSpec,
    mut input_rx: mpsc::Receiver<VncInputEvent>,
) {
    let mut attempt = 0u8;
    loop {
        // Never replay keys queued for a previous connection.
        while input_rx.try_recv().is_ok() {}
        let started = Instant::now();
        let result = super::connection::run(&shared, &runtime, &sid, &spec, &mut input_rx).await;
        match result {
            Err(Failure::Cancelled) | Ok(()) => {
                break;
            }
            Err(Failure::Permanent(message)) => {
                mark_failed(&shared, &runtime, &sid, message);
                break;
            }
            Err(Failure::Network(message)) => {
                if started.elapsed() >= Duration::from_mins(1) {
                    attempt = 0;
                }
                if !spec.options.reconnect.enabled || attempt >= spec.options.reconnect.max_attempts
                {
                    mark_disconnected(&shared, &runtime, &sid, Some(message));
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
                    tokio::select! {()=tokio::time::sleep(Duration::from_secs(1))=>{},_=input_rx.recv()=>{cancelled=true;break;}}
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
