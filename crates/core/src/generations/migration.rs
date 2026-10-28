use super::{GenerationsError, Result};
use crate::{Paths, State, atomic_write, fs, id};
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
    let lock = paths.runtime.join("daemon.lock");
    let legacy = options.open(&lock)?;
    lock_released_soon(&legacy)
        .map_err(|source| GenerationsError::LegacyUnresponsive { lock, source })?;
    migrate(paths, true)
}

/// How long a just-released legacy lock may stay held: a retiring daemon
/// still owns it while it exits, and a child forked meanwhile shares it
/// until that child's `exec`.
const LEGACY_LOCK_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Take the lock, retrying briefly while it is held.
fn lock_released_soon(lock: &fs::File) -> std::io::Result<()> {
    let start = std::time::Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error) if start.elapsed() >= LEGACY_LOCK_GRACE => return Err(error),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
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
    if legacy.sessions.iter().any(|s| s.lifecycle.live()) {
        return Err(GenerationsError::MigrationWaits);
    }
    if database.exists() {
        let conn = Connection::open(&database)?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 2 {
            return Err(GenerationsError::LegacyIncompatible { version });
        }
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
