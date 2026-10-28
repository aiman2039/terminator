//! Shared catalog and generation ownership. No operation here starts a session.
mod catalog;
mod error;
#[cfg(test)]
mod generations_tests;
mod migration;
mod ownership;
mod recovery;

pub use catalog::{
    CAPABILITY, CATALOG_VERSION, Catalog, Generation, Health, Status, build_identity, coordinate,
    coordinate_timeout, exists, workspace_paths,
};
pub use error::GenerationsError;
pub use migration::{migrate_after_legacy_exit, migrate_idle};
pub use ownership::{owner_for, validate_endpoint};
#[cfg(feature = "async-client")]
pub(crate) use recovery::unavailable_snapshot;
pub use recovery::{
    RETIRED_RETENTION, clear_owned, historical, mergeable_presence, prune_retired, recover_exited,
    saved, snapshot,
};

type Result<T, E = GenerationsError> = std::result::Result<T, E>;
