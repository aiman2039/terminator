//! Cached per-owner id sets used to route session requests.
//!
//! Routing must know which generation owns a session, but parsing every
//! owner's whole saved state on each request (one per VNC mouse move) is too
//! slow once upgrades leave old generations behind. Only the ids are kept, and
//! they are reused while the owner's store files are unchanged.
use super::Result;
use super::recovery::saved;
use crate::Paths;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, PoisonError},
    time::{Duration, SystemTime},
};

/// Files written within this window may change again without a visible
/// size or mtime change on coarse-timestamp filesystems; never trust them.
const RACY_WINDOW: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct OwnerIndex {
    pub sessions: HashSet<String>,
    pub vnc_sessions: HashSet<String>,
    pub notifications: HashSet<String>,
    pub terminal_notices: HashSet<String>,
}

/// Size and mtime of the SQLite store and its write-ahead log.
type Stamp = [Option<(u64, SystemTime)>; 2];

/// Owner data directory to the stamp its cached index was read under.
type Cache = HashMap<PathBuf, (Stamp, Arc<OwnerIndex>)>;

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

fn stamp(data: &Path) -> Stamp {
    [
        file_stamp(&data.join("state.sqlite3")),
        file_stamp(&data.join("state.sqlite3-wal")),
    ]
}

/// A stamp is trusted only when the store exists and no file changed
/// recently enough to hide a later write behind the same stamp.
fn settled(stamp: &Stamp, now: SystemTime) -> bool {
    stamp[0].is_some()
        && stamp.iter().flatten().all(|(_, modified)| {
            now.duration_since(*modified)
                .is_ok_and(|age| age >= RACY_WINDOW)
        })
}

/// The owner's id sets, read from disk only when its store changed. The stamp
/// is taken before the read, so a concurrent write is seen on the next call.
pub(super) fn owner_index(paths: &Paths) -> Result<Arc<OwnerIndex>> {
    let stamp = stamp(&paths.data);
    let trusted = settled(&stamp, SystemTime::now());
    if trusted
        && let Some((cached, index)) = CACHE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&paths.data)
        && *cached == stamp
    {
        return Ok(Arc::clone(index));
    }
    let state = saved(paths)?;
    let index = Arc::new(OwnerIndex {
        sessions: state.sessions.into_iter().map(|s| s.id).collect(),
        vnc_sessions: state.vnc_sessions.into_iter().map(|s| s.id).collect(),
        notifications: state.notifications.into_iter().map(|n| n.id).collect(),
        terminal_notices: state.terminal_notices.into_iter().map(|n| n.id).collect(),
    });
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if trusted {
        cache.insert(paths.data.clone(), (stamp, Arc::clone(&index)));
    } else {
        cache.remove(&paths.data);
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recently_written_store_is_not_trusted() {
        let now = SystemTime::now();
        let old = SystemTime::UNIX_EPOCH;
        assert!(settled(&[Some((1, old)), None], now));
        assert!(settled(&[Some((1, old)), Some((2, old))], now));
        assert!(!settled(&[Some((1, now)), None], now));
        assert!(!settled(&[Some((1, old)), Some((2, now))], now));
    }

    #[test]
    fn missing_store_is_not_trusted() {
        let now = SystemTime::now();
        assert!(!settled(&[None, Some((1, SystemTime::UNIX_EPOCH))], now));
    }

    #[test]
    fn changed_store_stamp_does_not_match_cached_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.sqlite3");
        std::fs::write(&db, b"a").unwrap();
        let before = stamp(dir.path());
        std::fs::write(&db, b"ab").unwrap();
        assert_ne!(before, stamp(dir.path()));
    }
}
