//! Why an async daemon request failed.
use crate::{
    FrameError, IpcError, PathsError, async_service::Failure, generations::GenerationsError,
    snapshot::SnapshotError, ssh::SshError, transport::TransportError,
};

/// Why an async daemon request failed, and whether its outcome is unknown.
#[derive(Debug, thiserror::Error)]
pub enum AsyncClientError {
    /// The request may have reached the daemon before the connection failed.
    #[error("Mutation outcome is uncertain; reconcile state before retrying")]
    UncertainMutation {
        #[source]
        source: Box<AsyncClientError>,
    },
    #[error("Session daemon connection deadline")]
    ConnectDeadline {
        socket: std::path::PathBuf,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("Session daemon unavailable")]
    Unavailable {
        #[source]
        source: TransportError,
    },
    #[error("IPC response deadline exceeded for {operation}")]
    ResponseDeadline {
        operation: &'static str,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("Snapshot exceeds the 256 MiB client limit; saved state is intact")]
    ResponseTooLarge { bytes: usize },
    #[error("Snapshot superseded by an active-owner change")]
    Superseded { generation: String },
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Generations(#[from] GenerationsError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    Failure(#[from] Failure),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AsyncClientError {
    #[must_use]
    pub fn is_uncertain_mutation(&self) -> bool {
        matches!(self, Self::UncertainMutation { .. })
    }
    /// The classified SSH failure the daemon reported, if any.
    #[must_use]
    pub fn ssh(&self) -> Option<&SshError> {
        match self {
            Self::Ipc(error) => error.ssh(),
            _ => None,
        }
    }
}
