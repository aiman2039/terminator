//! Agent usage worker: reads what the agent CLIs report about their quota
//! every 30 seconds while the Usage tool shows. Read-only file reads off
//! the GUI thread.
use crate::{Update, gui_services::Services};
use std::time::Duration;
use terminator_core::{
    agents::quota,
    async_service::{CancellationToken, OperationContext, Policy},
};

const PERIOD: Duration = Duration::from_secs(30);

/// `true` while the Usage tool shows.
pub type QuotaJobs = tokio::sync::watch::Sender<bool>;

/// `data` is the shared data folder where the status-line hook writes.
pub fn spawn(service: Services, data: std::path::PathBuf) -> QuotaJobs {
    let (sender, mut shown) = tokio::sync::watch::channel(false);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let mut context = OperationContext::new("quota", "agents".into(), Policy::ServiceLifetime);
    context.deadline = None;
    let handle = service.handle().clone();
    let _ = handle.submit(context, cancel, async move {
        loop {
            let polling = *shown.borrow_and_update();
            if polling {
                let data = data.clone();
                let read = service
                    .fs()
                    .run(&token, move || {
                        Ok::<_, terminator_core::async_service::Failure>(quota::read_all(&data))
                    })
                    .await
                    .unwrap_or_default();
                service.emit_read(Update::Quota(read)).await?;
            }
            tokio::select! {
                () = token.cancelled() => break,
                changed = shown.changed() => if changed.is_err() { break },
                () = tokio::time::sleep(PERIOD), if polling => {},
            }
        }
        Ok(Vec::new())
    });
    sender
}

/// Short text for one window: `5h 42% · resets in 1h 05m`.
pub fn window_text(window: &quota::QuotaWindow, now: u64) -> String {
    let used = format!("{} {:.0}%", window.label, window.used_percent);
    match window.resets_at {
        Some(at) if at > now => {
            let left = at.saturating_sub(now);
            let (days, hours, minutes) = (left / 86_400, left % 86_400 / 3600, left % 3600 / 60);
            if days > 0 {
                format!("{used} · resets in {days}d {hours}h")
            } else {
                format!("{used} · resets in {hours}h {minutes:02}m")
            }
        }
        Some(_) => format!("{used} · reset"),
        None => used,
    }
}
