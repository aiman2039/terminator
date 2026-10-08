use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use terminator_core::{Paths, Session, SessionKind};
pub(crate) const MAX_DOCUMENT: usize = 1024 * 1024;
pub(super) const INTERVAL: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Mode {
    Edit,
    #[default]
    Preview,
    Split,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edit",
            Self::Preview => "Preview",
            Self::Split => "Split",
        }
    }
}
pub(crate) fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdown" | "mkd" | "mkdn"
        )
    })
}
pub(crate) fn available(session: &Session) -> bool {
    session.kind == SessionKind::Editor
        && !session.review
        && session.lifecycle.live()
        && session.file.as_deref().is_some_and(supported)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub(super) session: String,
    pub(super) path: PathBuf,
    pub(super) socket: PathBuf,
}
impl Source {
    pub fn new(paths: &Paths, session: &Session) -> Self {
        Self {
            session: session.id.clone(),
            path: session.file.clone().unwrap_or_default(),
            socket: paths.editor_socket(&session.id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct Revision {
    pub(super) buffer: u64,
    pub(super) tick: u64,
    pub(super) modified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct Snapshot {
    pub(super) path: PathBuf,
    pub(super) text: String,
    pub(super) revision: Option<Revision>,
    #[serde(default)]
    pub(super) paused: bool,
}
#[cfg(test)]
#[cfg(unix)]
pub(super) fn read_source(
    source: &Source,
    previous: Option<&Snapshot>,
) -> Result<Option<Snapshot>> {
    if source.socket.exists() {
        let response = (|| -> Result<Option<serde_json::Value>> {
            let mut rpc =
                crate::nvim_rpc::Connection::connect(&source.socket, Duration::from_millis(400))?;
            // Fast requests still work at swap-file, hit-enter and input() prompts.
            // Ordinary evaluation would be deferred until the user answers them.
            let mode = rpc.call("nvim_get_mode", serde_json::json!([]), 4096)?;
            if mode["blocking"].as_bool().context("Invalid Neovim mode")? {
                return Ok(None);
            }
            let revision = previous
                .filter(|s| !s.paused)
                .and_then(|s| s.revision.as_ref());
            let arguments =
                serde_json::json!({"path":source.path, "previous":revision, "limit":MAX_DOCUMENT});
            let code = format!(
                "local _A = ...\n{}",
                include_str!("../markdown_snapshot.lua")
            );
            Ok(Some(rpc.call(
                "nvim_exec_lua",
                serde_json::json!([code, [arguments]]),
                MAX_DOCUMENT * 6 + 16384,
            )?))
        })();
        let value = match response {
            Ok(Some(value)) => value,
            Ok(None) => return paused_snapshot(source, previous).map(Some),
            Err(error) if crate::nvim_rpc::timed_out(&error) => {
                return paused_snapshot(source, previous).map(Some);
            }
            Err(error) => return Err(error.context("Could not read the live editor buffer")),
        };
        let value: serde_json::Value =
            serde_json::from_str(value.as_str().context("Invalid editor preview response")?)?;
        if let Some(error) = value["error"].as_str() {
            anyhow::bail!("{error}");
        }
        if value["unchanged"].as_bool() == Some(true) {
            return Ok(None);
        }
        let snapshot: Snapshot = serde_json::from_value(value)?;
        ensure!(
            snapshot.text.len() <= MAX_DOCUMENT,
            "Markdown preview is limited to 1 MiB"
        );
        return Ok(Some(snapshot));
    }
    read_saved(source).map(Some)
}
#[cfg(test)]
#[cfg(unix)]
fn paused_snapshot(source: &Source, previous: Option<&Snapshot>) -> Result<Snapshot> {
    // Never replace an observed unsaved buffer with older disk contents.
    let mut snapshot = match previous.filter(|s| s.revision.is_some()) {
        Some(snapshot) => snapshot.clone(),
        None => read_saved(source)?,
    };
    snapshot.paused = true;
    Ok(snapshot)
}
#[cfg(test)]
#[cfg(unix)]
fn read_saved(source: &Source) -> Result<Snapshot> {
    read_saved_cancel(
        source,
        &terminator_core::async_service::CancellationToken::new(),
    )
}
pub(super) fn read_saved_cancel(
    source: &Source,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<Snapshot> {
    // Custom terminal editors have no Neovim RPC. Clearly label their saved-file view.
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Avoid blocking on FIFOs; Windows has no equivalent flag.
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(&source.path).context("Open Markdown file")?;
    ensure!(
        file.metadata()?.is_file(),
        "Markdown preview requires a regular file"
    );
    let bytes = terminator_core::async_service::read_chunks(file, MAX_DOCUMENT + 1, cancel)?;
    ensure!(
        bytes.len() <= MAX_DOCUMENT,
        "Markdown preview is limited to 1 MiB"
    );
    Ok(Snapshot {
        path: source.path.clone(),
        text: String::from_utf8(bytes).context("Markdown file is not UTF-8")?,
        revision: None,
        paused: false,
    })
}
