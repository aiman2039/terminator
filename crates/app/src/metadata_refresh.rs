//! Latest selection, bounded asynchronous metadata work, independent of editor IPC.
use crate::{Update, gui_services::Services};
use std::{path::PathBuf, time::Duration};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub cwd: PathBuf,
    pub identity: Option<(u32, u64)>,
    pub include_pr: bool,
    pub generation: u64,
}
pub fn spawn(service: Services) -> tokio::sync::watch::Sender<Option<Request>> {
    let (sender, mut requests) = tokio::sync::watch::channel::<Option<Request>>(None);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let mut context =
        OperationContext::new("metadata", "selection".into(), Policy::ServiceLifetime);
    context.deadline = None;
    let handle = service.handle().clone();
    let _ = handle.submit(context, cancel, async move {
        let mut cache = terminator_core::metadata::Cache::default();
        loop {
            let request = requests.borrow_and_update().clone();
            let Some(request) = request else {
                tokio::select! { _ = token.cancelled() => break, result = requests.changed() => if result.is_err() { break } }
                continue;
            };
            let mut collecting = cache.clone();
            let result = tokio::select! {
                _ = token.cancelled() => break,
                result = requests.changed() => { if result.is_err() { break; } continue; }
                result = collecting.collect_async(service.processes(), service.fs(), &request.cwd, request.identity, request.include_pr) => result,
            };
            cache = collecting;
            service.emit_read(Update::Metadata(request.generation, result)).await?;
            tokio::select! {
                _ = token.cancelled() => break,
                result = requests.changed() => if result.is_err() { break; },
                _ = tokio::time::sleep(Duration::from_secs(3)) => {},
            }
        }
        Ok(Vec::new())
    });
    sender
}
