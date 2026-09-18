//! Authenticated GUI socket, supervised async clients; never join from window teardown.
use crate::{Update, gui_services::Services};
use anyhow::{Result, ensure};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use terminator_core::{
    Paths,
    async_service::{CancellationToken, OperationContext, Policy},
    ui_control::{Envelope, Response},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub struct Server {
    cancel: CancellationToken,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub fn spawn(paths: Paths, service: Services) -> Result<Server> {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let mut context =
        OperationContext::new("gui-control", "listener".into(), Policy::ServiceLifetime);
    context.deadline = None;
    let handle = service.handle().clone();
    handle.submit(context, cancel.clone(), async move {
        let socket = paths.runtime.join("gui.sock");
        let bind = socket.clone();
        let listener = service
            .client()
            .catalog
            .run(&token, move || {
                let _ = fs::remove_file(&bind);
                let listener = std::os::unix::net::UnixListener::bind(&bind)?;
                fs::set_permissions(bind, fs::Permissions::from_mode(0o600))?;
                listener.set_nonblocking(true)?;
                Ok(listener)
            })
            .await?;
        let listener = tokio::net::UnixListener::from_std(listener)?;
        let mut clients = FuturesUnordered::new();
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                accepted = listener.accept(), if clients.len() < 16 => {
                    let (stream, _) = accepted?;
                    let service = service.clone(); let paths = paths.clone();
                    clients.push(async move { serve(stream, service, paths).await }.boxed());
                }
                Some(_) = clients.next(), if !clients.is_empty() => {}
            }
        }
        drop(clients);
        drop(listener);
        service
            .client()
            .catalog
            .run(&CancellationToken::new(), move || {
                let _ = fs::remove_file(socket);
                Ok(())
            })
            .await?;
        Ok(Vec::new())
    })?;
    Ok(Server { cancel })
}
async fn serve(stream: tokio::net::UnixStream, service: Services, paths: Paths) -> Result<()> {
    serve_for(stream, service, paths, Duration::from_secs(8)).await
}
async fn serve_for(
    stream: tokio::net::UnixStream,
    service: Services,
    paths: Paths,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    tokio::time::timeout_at(
        deadline.into(),
        serve_until(stream, service, paths, deadline),
    )
    .await
    .map_err(|_| anyhow::anyhow!("GUI response deadline exceeded"))?
}
async fn serve_until(
    mut stream: tokio::net::UnixStream,
    service: Services,
    paths: Paths,
    deadline: Instant,
) -> Result<()> {
    let result: Result<serde_json::Value> = async {
        let bytes = tokio::time::timeout(Duration::from_secs(3), async {
            let size = stream.read_u32().await? as usize;
            ensure!(size <= terminator_core::MAX_FRAME, "IPC frame too large");
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).await?;
            Ok::<_, anyhow::Error>(bytes)
        })
        .await??;
        let envelope: Envelope = service
            .cpu()
            .run(&CancellationToken::new(), move || {
                Ok(serde_json::from_slice(&bytes)?)
            })
            .await?;
        let token = service
            .client()
            .catalog
            .run(&CancellationToken::new(), move || paths.token())
            .await?;
        ensure!(
            envelope.version == 1 && envelope.auth == token,
            "GUI authentication failed"
        );
        envelope.request.validate()?;
        if !matches!(
            envelope.request,
            terminator_core::ui_control::Request::Snapshot
                | terminator_core::ui_control::Request::Ping
        ) && let terminator_core::Response::State(state) = service
            .client()
            .rpc(terminator_core::Request::Snapshot)
            .await?
        {
            service.emit(Update::State(state)).await?;
        }
        let (reply, response) = tokio::sync::oneshot::channel();
        let deadline = deadline.min(Instant::now() + Duration::from_secs(5));
        service
            .emit(Update::AsyncUiRequest(envelope.request, reply, deadline))
            .await?;
        tokio::time::timeout_at(deadline.into(), response)
            .await??
            .map_err(anyhow::Error::msg)
    }
    .await;
    let response = match result {
        Ok(value) => Response {
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            result: None,
            error: Some(error.to_string()),
        },
    };
    let bytes = service
        .cpu()
        .run(&CancellationToken::new(), move || {
            Ok(serde_json::to_vec(&response)?)
        })
        .await?;
    ensure!(
        bytes.len() <= terminator_core::MAX_FRAME,
        "GUI response too large"
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&bytes).await
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn catalogue_delay_cannot_execute_a_gui_request_after_its_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::at(directory.path().into());
        paths.init().unwrap();
        fs::write(paths.auth(), "fixture").unwrap();
        let (updates, _) = std::sync::mpsc::channel();
        let (service, mut owner) =
            Services::new(paths.clone(), eframe::egui::Context::default(), updates).unwrap();
        let catalog = service.client().catalog.clone();
        let (started, entered) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = tokio::spawn(async move {
            catalog
                .run(&CancellationToken::new(), move || {
                    let _ = started.send(());
                    blocked.recv().unwrap();
                    Ok(())
                })
                .await
        });
        entered.await.unwrap();
        let listener = tokio::net::UnixListener::bind(paths.runtime.join("deadline.sock")).unwrap();
        let mut client = tokio::net::UnixStream::connect(paths.runtime.join("deadline.sock"))
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let task = tokio::spawn(serve_for(server, service, paths, Duration::from_millis(80)));
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            auth: "fixture".into(),
            request: terminator_core::ui_control::Request::Ping,
        })
        .unwrap();
        client.write_u32(bytes.len() as u32).await.unwrap();
        client.write_all(&bytes).await.unwrap();
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
        release.send(()).unwrap();
        worker.await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            owner.events.try_recv().is_err(),
            "Expired request reached the GUI"
        );
    }
}
