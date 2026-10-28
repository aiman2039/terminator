//! Failures of the generation catalog, ownership routing, recovery, and migration.

/// Why a generation catalog operation, owner lookup, or migration failed.
#[derive(Debug, thiserror::Error)]
pub enum GenerationsError {
    #[error("Session service is busy; retry the action")]
    Busy { waited: std::time::Duration },
    #[error("Unsupported generation catalog version {version}")]
    UnsupportedCatalog { version: u32 },
    #[error("Catalog already exists")]
    CatalogExists,
    #[error("Legacy sessions must finish before migration")]
    LegacySessionsLive,
    #[error("Incompatible generation")]
    Incompatible {
        owner: String,
        protocol: u32,
        catalog: u32,
    },
    #[error("Unknown owner")]
    UnknownOwner { owner: String },
    #[error("Owner has already started")]
    AlreadyStarted { owner: String },
    #[error("Another recovery is already in progress")]
    RecoveryInProgress,
    #[error("Session creation is frozen for recovery")]
    CreationFrozen { owner: String },
    #[error("Candidate is not prepared")]
    NotPrepared { owner: String },
    #[error("Live owner is incompatible")]
    LiveOwnerIncompatible { candidate: String },
    #[error("Unknown active owner")]
    UnknownActiveOwner { owner: String },
    #[error("Only the active generation may save workspace data")]
    NotActive { generation: String },
    #[error("Session owner is unavailable")]
    OwnerUnavailable { request: &'static str },
    #[error("Unknown close owner")]
    UnknownCloseOwner { generation: String },
    #[error("No active generation")]
    NoActiveGeneration,
    #[error("Active owner is unregistered")]
    ActiveUnregistered { generation: String },
    #[error("Session owner is unavailable; generation {generation} store is unreadable")]
    UnreadableStore {
        generation: String,
        #[source]
        source: Box<GenerationsError>,
    },
    #[error("Generation is not registered in this catalog")]
    Unregistered { generation: String },
    #[error(
        "Inherited generation/catalog routing does not match this data/runtime directory; clear inherited TERMINATOR_CATALOG_DATA, TERMINATOR_CATALOG_RUNTIME and TERMINATOR_GENERATION for isolated development"
    )]
    RoutingMismatch { generation: String },
    #[error("Running daemon is unresponsive or incompatible; its sessions have been preserved")]
    LegacyUnresponsive {
        lock: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Migration waits for legacy sessions to finish")]
    MigrationWaits,
    #[error("Legacy database is incompatible")]
    LegacyIncompatible { version: u32 },
    #[error(transparent)]
    Remote(#[from] crate::remote::RemoteError),
    #[error(transparent)]
    Write(#[from] crate::AtomicWriteError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Revision(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
