//! One-time import of a legacy single-daemon store into a new catalog.
use super::*;

impl Catalog {
    /// Caller holds the legacy exclusive daemon lock and coordination lock.
    /// Publication is an atomic rename after a complete, independently readable import.
    fn publish_catalog(paths: &Paths, temporary: &Path) -> Result<()> {
        fs::OpenOptions::new()
            .write(true)
            .open(temporary)?
            .sync_all()?;
        fs::rename(temporary, paths.data.join("catalog.sqlite3"))?;
        // Windows does not support opening a directory with File::open.
        #[cfg(unix)]
        fs::File::open(&paths.data)?.sync_all()?;
        Ok(())
    }

    pub fn import(paths: &Paths, legacy: &State) -> Result<()> {
        if exists(paths) {
            return Err(GenerationsError::CatalogExists);
        }
        if legacy.sessions.iter().any(|s| s.lifecycle.live()) {
            return Err(GenerationsError::LegacySessionsLive);
        }
        let temporary = paths.data.join(format!("catalog-{}.tmp", id()));
        let conn = Self::create_import(&temporary, legacy)?;
        // Import each historical owner independently, preserving UUIDs and history.
        let owners: BTreeSet<_> = legacy
            .sessions
            .iter()
            .map(|s| s.generation.clone())
            .collect();
        for owner in owners {
            let record = Self::import_owner(paths, legacy, owner)?;
            conn.execute(
                "INSERT INTO generations VALUES(?1,?2)",
                params![record.id, serde_json::to_string(&record)?],
            )?;
        }
        drop(conn);
        Self::publish_catalog(paths, &temporary)
    }

    /// The unpublished catalog database holding the legacy shared workspace.
    fn create_import(temporary: &Path, legacy: &State) -> Result<Connection> {
        let conn = Connection::open(temporary)?;
        conn.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE workspace(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL,revision INTEGER NOT NULL); CREATE TABLE generations(id TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE control(id INTEGER PRIMARY KEY CHECK(id=1),active TEXT,frozen INTEGER NOT NULL DEFAULT 0, freeze_pid INTEGER); INSERT INTO control(id) VALUES(1); PRAGMA user_version=1;")?;
        let mut shared = legacy.clone();
        clear_owned(&mut shared);
        conn.execute(
            "INSERT INTO workspace VALUES(1,?1,?2)",
            params![
                serde_json::to_string(&shared)?,
                i64::try_from(legacy.revision)?
            ],
        )?;
        Ok(conn)
    }

    /// Archives one legacy owner's records and history as a retired generation.
    fn import_owner(paths: &Paths, legacy: &State, owner: String) -> Result<Generation> {
        let data = paths.data.join("generations").join(id());
        let archive_paths = Paths::at(data.clone());
        fs::create_dir_all(archive_paths.history_dir())?;
        #[cfg(unix)]
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
        let (archive, ids) = owner_archive(legacy, &owner);
        let db = Connection::open(data.join("state.sqlite3"))?;
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;")?;
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&archive)?],
        )?;
        copy_owner_history(paths, &archive_paths, &ids)?;
        Ok(Generation {
            id: owner,
            data,
            runtime: archive_paths.runtime,
            version: legacy.daemon_version.clone().unwrap_or_default(),
            build: "legacy import".into(),
            protocol: PROTOCOL_VERSION,
            catalog: CATALOG_VERSION,
            status: Status::Retired,
            pid: None,
        })
    }
}

/// `legacy` narrowed to `owner`'s sessions and their records, plus those session IDs.
fn owner_archive(legacy: &State, owner: &str) -> (State, BTreeSet<String>) {
    let mut archive = legacy.clone();
    owner.clone_into(&mut archive.generation);
    archive.sessions.retain(|s| s.generation == owner);
    let ids: BTreeSet<_> = archive.sessions.iter().map(|s| s.id.clone()).collect();
    archive.agents.retain(|a| ids.contains(&a.session_id));
    archive
        .notifications
        .retain(|n| ids.contains(&n.session_id));
    archive
        .terminal_notices
        .retain(|n| ids.contains(&n.session_id));
    archive.projects.clear();
    archive.worktrees.clear();
    (archive, ids)
}

/// Copies and syncs the legacy history files of the sessions in `ids`.
fn copy_owner_history(paths: &Paths, archive: &Paths, ids: &BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(paths.history_dir())? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_file()
            || !ids.iter().any(|sid| name.starts_with(&format!("{sid}-")))
        {
            continue;
        }
        let destination = archive.history_dir().join(entry.file_name());
        fs::copy(entry.path(), &destination)?;
        fs::OpenOptions::new()
            .write(true)
            .open(destination)?
            .sync_all()?;
    }
    Ok(())
}
