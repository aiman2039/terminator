use crate::{Context, Paths, Result, State, atomic_write, ensure, fs, id};
use fs2::FileExt;
use rusqlite::Connection;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use super::catalog::{Catalog, coordinate, exists};
use super::recovery::saved;

/// The caller holds the exclusive legacy lock. Original database and history are
/// retained; a consistent `SQLite` backup precedes the older-binary version guard.
pub fn migrate_idle(paths: &Paths) -> Result<()> {
    migrate(paths, false)
}

/// A legacy daemon holds this lock for its entire lifetime. Acquiring it proves
/// that service has exited, even when its last persisted records still say live.
/// Reconcile records only; never signal, adopt or rerun their recorded PIDs.
pub fn migrate_after_legacy_exit(paths: &Paths) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let legacy = options.open(paths.runtime.join("daemon.lock"))?;
    legacy.try_lock_exclusive().context(
        "Running daemon is unresponsive or incompatible; its sessions have been preserved",
    )?;
    migrate(paths, true)
}

fn migrate(paths: &Paths, reconcile_exited: bool) -> Result<()> {
    let _coordination = coordinate(paths)?;
    if !paths.auth().exists() {
        atomic_write(&paths.auth(), id().as_bytes())?;
    }
    if exists(paths) {
        return Ok(());
    }
    let database = paths.data.join("state.sqlite3");
    let mut legacy = if database.exists() {
        saved(paths)?
    } else {
        State::default()
    };
    if reconcile_exited && legacy.sessions.iter().any(|s| s.lifecycle.live()) {
        legacy.recover();
    }
    ensure!(
        !legacy.sessions.iter().any(|s| s.lifecycle.live()),
        "Migration waits for legacy sessions to finish"
    );
    if database.exists() {
        let conn = Connection::open(&database)?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 2, "Legacy database is incompatible");
        let backup = paths.data.join("state.before-generations.sqlite3");
        if !backup.exists() {
            conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        }
        conn.execute_batch("PRAGMA user_version=2;")?;
    } else {
        let conn = Connection::open(&database)?;
        conn.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=2;")?;
        conn.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&legacy)?],
        )?;
    }
    Catalog::import(paths, &legacy)
}
