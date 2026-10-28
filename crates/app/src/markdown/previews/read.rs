use super::super::source::{MAX_DOCUMENT, Snapshot, Source, read_saved_cancel};
use anyhow::{Context, Result, ensure};
use std::{sync::Arc, time::Duration};
use terminator_core::async_service::CancellationToken;

pub(crate) struct ReadSourceAsyncInput<'a> {
    pub(crate) service: &'a crate::gui_services::Services,
    pub(crate) source: &'a Source,
    pub(crate) previous: Option<Arc<Snapshot>>,
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) force: bool,
}

pub(super) async fn read_source_async(input: ReadSourceAsyncInput<'_>) -> Result<Option<Snapshot>> {
    let ReadSourceAsyncInput {
        service,
        source,
        previous,
        cancel,
        force,
    } = input;
    SourceRead {
        service,
        source,
        previous,
        cancel,
        force,
    }
    .read()
    .await
}

/// One read of a watched Markdown source: the live Neovim buffer when the
/// editor answers, else the saved file (marked paused).
struct SourceRead<'a> {
    service: &'a crate::gui_services::Services,
    source: &'a Source,
    previous: Option<Arc<Snapshot>>,
    cancel: &'a CancellationToken,
    force: bool,
}

impl SourceRead<'_> {
    async fn read(self) -> Result<Option<Snapshot>> {
        if self.socket_live().await? {
            return self.read_live().await;
        }
        if self
            .previous
            .as_deref()
            .is_some_and(|s| s.revision.is_some())
        {
            return self.paused().await.map(Some);
        }
        let source = self.source.clone();
        let operation = self.cancel.clone();
        self.service
            .fs()
            .run(self.cancel, move || read_saved_cancel(&source, &operation))
            .await
            .map(Some)
    }

    // Windows Neovim listens on a named pipe with no filesystem record, so
    // the existence probe is Unix-only; on Windows the connect attempt
    // decides, falling back to saved content on timeout.
    #[cfg(unix)]
    async fn socket_live(&self) -> Result<bool> {
        let socket = self.source.socket.clone();
        self.service
            .fs()
            .run(self.cancel, move || anyhow::Ok(socket.exists()))
            .await
    }

    #[cfg(windows)]
    async fn socket_live(&self) -> Result<bool> {
        Ok(true)
    }

    async fn read_live(&self) -> Result<Option<Snapshot>> {
        let value = match self.query_live().await {
            Ok(Some(value)) => value,
            Ok(None) => return self.paused().await.map(Some),
            Err(error) if crate::nvim_rpc::timed_out(&error) => {
                return self.paused().await.map(Some);
            }
            Err(error) => return Err(error.context("Could not read the live editor buffer")),
        };
        let value: serde_json::Value =
            serde_json::from_str(value.as_str().context("Invalid editor preview response")?)?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            anyhow::bail!("{error}");
        }
        if value.get("unchanged").and_then(serde_json::Value::as_bool) == Some(true) {
            return Ok(None);
        }
        let snapshot: Snapshot = serde_json::from_value(value)?;
        ensure!(
            snapshot.text.len() <= MAX_DOCUMENT,
            "Markdown preview is limited to 1 MiB"
        );
        Ok(Some(snapshot))
    }

    /// Ask Neovim for the buffer; `None` while it is blocked at a prompt.
    async fn query_live(&self) -> Result<Option<serde_json::Value>> {
        let source = self.source;
        let mut rpc = crate::nvim_rpc::AsyncConnection::connect(
            &source.socket,
            Duration::from_millis(400),
            self.service.cpu().clone(),
        )
        .await?;
        // Fast requests still work at swap-file, hit-enter and input() prompts.
        // Ordinary evaluation would be deferred until the user answers them.
        let mode = rpc
            .call("nvim_get_mode", serde_json::json!([]), 4096)
            .await?;
        if mode
            .get("blocking")
            .and_then(serde_json::Value::as_bool)
            .context("Invalid Neovim mode")?
        {
            return Ok(None);
        }
        let revision = self
            .previous
            .as_deref()
            .filter(|s| !self.force && !s.paused)
            .and_then(|s| s.revision.as_ref());
        let arguments =
            serde_json::json!({"path":source.path, "previous":revision, "limit":MAX_DOCUMENT});
        let code = format!(
            "local _A = ...\n{}",
            include_str!("../../markdown_snapshot.lua")
        );
        Ok(Some(
            rpc.call(
                "nvim_exec_lua",
                serde_json::json!([code, [arguments]]),
                MAX_DOCUMENT * 6 + 16384,
            )
            .await?,
        ))
    }

    /// The last live snapshot, or the saved file, marked paused.
    async fn paused(&self) -> Result<Snapshot> {
        let operation = self.cancel.clone();
        let source = self.source.clone();
        let previous = self.previous.clone();
        self.service
            .fs()
            .run(self.cancel, move || {
                let mut snapshot = match previous.as_deref().filter(|s| s.revision.is_some()) {
                    Some(snapshot) => snapshot.clone(),
                    None => read_saved_cancel(&source, &operation)?,
                };
                snapshot.paused = true;
                Ok(snapshot)
            })
            .await
    }
}
