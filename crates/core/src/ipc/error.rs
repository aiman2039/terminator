//! Failures of the local daemon protocol: framing, routing, and daemon replies.

/// Why a frame could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("IPC frame too large")]
    TooLarge { bytes: usize },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Why a daemon request failed, before or after it reached the daemon.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("Request rejected before execution; use active generation {generation}")]
    Redirected { generation: String },
    /// The daemon answered `Response::Error`; the text is the daemon's.
    #[error("{message}")]
    Daemon { message: String },
    /// The daemon answered `Response::SshError`.
    #[error(transparent)]
    Ssh(crate::ssh::SshError),
    #[error("Redirect names an unregistered owner")]
    UnregisteredRedirect { generation: String },
    #[error("Active service changed repeatedly before execution; retry the operation")]
    Unstable { request: &'static str },
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Snapshot(#[from] crate::snapshot::SnapshotError),
    #[error(transparent)]
    Transport(#[from] crate::transport::TransportError),
    #[error(transparent)]
    Paths(#[from] crate::PathsError),
    #[error(transparent)]
    Generations(#[from] crate::generations::GenerationsError),
}

impl IpcError {
    /// The classified SSH failure the daemon reported, if any.
    #[must_use]
    pub fn ssh(&self) -> Option<&crate::ssh::SshError> {
        match self {
            Self::Ssh(error) => Some(error),
            _ => None,
        }
    }
}
