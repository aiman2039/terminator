//! Reading the stored state row: what counts as damage (quarantined) and
//! what is a state from a newer version (kept, the daemon refuses to start).
use anyhow::Result;
use terminator_core::State;

/// Only damaged content (SQLite corrupt/not-a-database, bad JSON
/// syntax) counts. Well-formed JSON that does not fit this build's
/// types (an unknown variant or field type, as after a downgrade) was
/// written by a newer version: it fails without quarantine, so going
/// back to the newer build still finds every session.
pub(super) fn corrupt(error: &anyhow::Error) -> bool {
    use rusqlite::{Error::SqliteFailure, ErrorCode};
    error.chain().any(|e| {
        e.downcast_ref::<serde_json::Error>()
            .is_some_and(|e| !e.is_data())
            || e.downcast_ref::<rusqlite::Error>().is_some_and(|e| {
                matches!(e, SqliteFailure(f, _)
                    if matches!(f.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase))
            })
    })
}

/// Decode the stored state row, naming why it failed.
pub(super) fn state(json: &str) -> Result<State> {
    serde_json::from_str(json).map_err(|error| {
        let why = if error.is_data() {
            "Stored state does not fit this version (written by a newer Terminator?); \
             it is kept unchanged"
        } else {
            "Stored state is not valid JSON"
        };
        anyhow::Error::new(error).context(why)
    })
}
