use crate::{Context, Paths, Request, Result, bail, ensure, fs};
use fs2::FileExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use super::catalog::{Catalog, Generation, Status, exists};
use super::recovery::saved;

pub(super) fn process_alive(pid: u32) -> bool {
    crate::signals::process_alive(pid)
}

pub(super) fn owner_is_serving(root: &Paths, owner: &Generation) -> Result<bool> {
    Ok(
        Catalog::open(root)?.active()?.as_deref() == Some(owner.id.as_str())
            && crate::transport::probe_ipc(&owner.paths().socket()),
    )
}

/// `None` means the owner is still live. A missing runtime is death: a reused PID
/// must not keep that generation's sessions attached.
pub(super) fn claim_dead_owner(owner: &Generation, pid: u32) -> Result<Option<std::fs::File>> {
    let path = owner.runtime.join("daemon.lock");
    match fs::OpenOptions::new().write(true).open(&path) {
        Ok(lock) => {
            if lock.try_lock_exclusive().is_err() || process_alive(pid) {
                return Ok(None);
            }
            Ok(Some(lock))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if owner.status == Status::Prepared || owner_socket_open(owner) {
                return Ok(None);
            }
            fs::create_dir_all(&owner.runtime)?;
            let mut options = fs::OpenOptions::new();
            options.create(true).truncate(false).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            let lock = options.open(&path)?;
            if lock.try_lock_exclusive().is_err() {
                return Ok(None);
            }
            Ok(Some(lock))
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn owner_socket_open(owner: &Generation) -> bool {
    crate::transport::probe_ipc(&owner.paths().socket())
}

pub fn owner_for(paths: &Paths, request: &Request) -> Result<Paths> {
    if !exists(paths) {
        return Ok(paths.clone());
    }
    let catalog = Catalog::open(paths)?;
    let owners = catalog.generations()?;
    let session = match request {
        Request::Stop { session }
        | Request::Rename { session, .. }
        | Request::Remove { session }
        | Request::Focus { session }
        | Request::Cwd { session, .. }
        | Request::Attach { session, .. }
        | Request::History { session }
        | Request::EditorCompare { session }
        | Request::EditorSave { session }
        | Request::EditorStatus { session }
        | Request::Screen { session }
        | Request::TerminalNotify { session, .. }
        | Request::ShellCommand { session }
        | Request::ShellPrompt { session, .. }
        | Request::ClearHistory {
            session: Some(session),
        } => Some(session.as_str()),
        Request::Hook(e) => Some(e.terminal_session_id.as_str()),
        _ => None,
    };
    if session.is_some()
        || matches!(
            request,
            Request::Notice { .. } | Request::DismissTerminalNotice { .. }
        )
    {
        for owner in owners {
            if owner.status == Status::Prepared {
                continue;
            }
            let state = saved(&owner.paths())?;
            let matches = match request {
                Request::Notice { id, .. } => state.notifications.iter().any(|n| &n.id == id),
                Request::DismissTerminalNotice { id } => {
                    state.terminal_notices.iter().any(|n| &n.id == id)
                }
                _ => state
                    .sessions
                    .iter()
                    .any(|s| Some(s.id.as_str()) == session),
            };
            if matches {
                return Ok(owner.paths());
            }
        }
        bail!("Session owner is unavailable");
    }
    if let Request::CloseIdleSessions { generation, .. } = request {
        return owners
            .into_iter()
            .find(|g| &g.id == generation)
            .map(|g| g.paths())
            .context("Unknown close owner");
    }
    let active = catalog.active()?.context("No active generation")?;
    owners
        .into_iter()
        .find(|g| g.id == active)
        .map(|g| g.paths())
        .context("Active owner is unregistered")
}

/// Reject inherited owner routing before a daemon can write another catalog.
pub fn validate_endpoint(root: &Paths, paths: &Paths, generation: &str) -> Result<()> {
    let owner = Catalog::open(root)?
        .generations()?
        .into_iter()
        .find(|g| g.id == generation)
        .context("Generation is not registered in this catalog")?;
    ensure!(
        owner.data.canonicalize()? == paths.data.canonicalize()?
            && owner.runtime.canonicalize()? == paths.runtime.canonicalize()?,
        "Inherited generation/catalog routing does not match this data/runtime directory; clear inherited TERMINATOR_CATALOG_DATA, TERMINATOR_CATALOG_RUNTIME and TERMINATOR_GENERATION for isolated development"
    );
    Ok(())
}
