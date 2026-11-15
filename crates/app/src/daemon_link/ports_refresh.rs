//! Ports tool worker: lists the listening ports of the selected project's
//! sessions every few seconds while the tool shows, and stops one process
//! on an explicit, confirmed request.
use crate::{Update, gui_services::Services};
use std::time::Duration;
use terminator_core::{
    async_service::{CancellationToken, OperationContext, Policy},
    metadata::{self, SessionPort},
};

/// Sessions to scan: session id, (PID, start time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub sessions: Vec<(String, (u32, u64))>,
    pub generation: u64,
}

#[derive(Debug)]
pub enum PortsUpdate {
    Listed(u64, Result<Vec<SessionPort>, String>),
    Stopped(SessionPort, Result<(), String>),
}

pub struct PortsJobs {
    pub list: tokio::sync::watch::Sender<Option<Request>>,
    pub stop: tokio::sync::mpsc::UnboundedSender<SessionPort>,
}

pub fn spawn(service: Services) -> PortsJobs {
    let (list, requests) = tokio::sync::watch::channel::<Option<Request>>(None);
    let (stop, stops) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let mut context = OperationContext::new("ports", "selection".into(), Policy::ServiceLifetime);
    context.deadline = None;
    let handle = service.handle().clone();
    let _ = handle.submit(context, cancel, async move {
        run(service, requests, stops, token).await;
        Ok(Vec::new())
    });
    PortsJobs { list, stop }
}

async fn run(
    service: Services,
    mut requests: tokio::sync::watch::Receiver<Option<Request>>,
    mut stops: tokio::sync::mpsc::UnboundedReceiver<SessionPort>,
    token: CancellationToken,
) {
    loop {
        let request = requests.borrow_and_update().clone();
        let polling = request.is_some();
        if let Some(Request {
            sessions,
            generation,
        }) = request
        {
            let listed = metadata::session_ports(service.processes(), service.fs(), sessions)
                .await
                .map_err(|error| error.to_string());
            let update = PortsUpdate::Listed(generation, listed);
            if service.emit_read(Update::Ports(update)).await.is_err() {
                return;
            }
        }
        tokio::select! {
            () = token.cancelled() => return,
            changed = requests.changed() => if changed.is_err() { return },
            () = tokio::time::sleep(Duration::from_secs(3)), if polling => {},
            port = stops.recv() => {
                let Some(port) = port else { return };
                let (pid, started) = (port.pid, port.started);
                let stopped = service
                    .fs()
                    .run(&token, move || metadata::stop_port_process(pid, started))
                    .await
                    .map_err(|error| error.to_string());
                if service.emit_read(Update::Ports(PortsUpdate::Stopped(port, stopped))).await.is_err() {
                    return;
                }
            }
        }
    }
}
