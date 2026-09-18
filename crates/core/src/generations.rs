//! Shared catalog and generation ownership. No operation here starts a session.
use crate::*;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, params};

pub const CAPABILITY: &str = "daemon-generations-v1";
pub const CATALOG_VERSION: u32 = 1;

/// Diagnostic identity for an immutable executable set. Verification additionally
/// compares complete files; this fingerprint is not a cryptographic trust check.
pub fn build_identity(bin: &Path) -> Result<String> {
    use std::hash::Hasher;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for name in ["terminator-daemon", "terminator-hook"] {
        let mut file = fs::File::open(bin.join(name))?;
        hash.write_u64(file.metadata()?.len());
        let mut bytes = [0; 65536];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.write(&bytes[..count]);
        }
    }
    Ok(format!("{:016x}", hash.finish()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Prepared,
    Active,
    Draining,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generation {
    pub id: String,
    pub data: PathBuf,
    pub runtime: PathBuf,
    pub version: String,
    pub build: String,
    pub protocol: u32,
    pub catalog: u32,
    pub status: Status,
    #[serde(default)]
    pub pid: Option<u32>,
}

impl Generation {
    pub fn paths(&self) -> Paths {
        Paths {
            data: self.data.clone(),
            runtime: self.runtime.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub owner: Generation,
    pub revision: u64,
    pub live_sessions: usize,
    pub error: Option<String>,
    pub capabilities: Vec<String>,
    pub helper: Option<PathBuf>,
}

pub fn workspace_paths(paths: &Paths) -> Result<Paths> {
    let marker = paths.data.join("workspace.json");
    if marker.exists() {
        return Ok(serde_json::from_slice(&fs::read(marker)?)?);
    }
    Ok(paths.clone())
}

pub fn exists(paths: &Paths) -> bool {
    catalog_version(paths).is_some_and(|version| version > 0)
}

fn catalog_version(paths: &Paths) -> Option<u32> {
    let path = paths.data.join("catalog.sqlite3");
    if path.metadata().ok()?.len() == 0 {
        return None;
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()?
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .ok()
}

/// All callers acquire this before activation or admitting a creation/mutation.
/// Each open has its own file description, so threads are serialized too.
pub fn coordinate(paths: &Paths) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(paths.data.join("coordination.lock"))?;
    file.lock_exclusive()?;
    Ok(file)
}

pub struct Catalog(Connection);
impl Catalog {
    pub fn open(paths: &Paths) -> Result<Self> {
        let conn = Connection::open_with_flags(
            paths.data.join("catalog.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        conn.busy_timeout(Duration::from_secs(3))?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version == CATALOG_VERSION,
            "Unsupported generation catalog version {version}"
        );
        Ok(Self(conn))
    }

    /// Caller holds the legacy exclusive daemon lock and coordination lock.
    /// Publication is an atomic rename after a complete, independently readable import.
    pub fn import(paths: &Paths, legacy: &State) -> Result<()> {
        ensure!(!exists(paths), "Catalog already exists");
        ensure!(
            !legacy.sessions.iter().any(|s| s.lifecycle.live()),
            "Legacy sessions must finish before migration"
        );
        let temporary = paths.data.join(format!("catalog-{}.tmp", id()));
        let conn = Connection::open(&temporary)?;
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
        // Import each historical owner independently, preserving UUIDs and history.
        let owners: BTreeSet<_> = legacy
            .sessions
            .iter()
            .map(|s| s.generation.clone())
            .collect();
        for owner in owners {
            let data = paths.data.join("generations").join(id());
            let archive_paths = Paths::at(data.clone());
            fs::create_dir_all(archive_paths.history_dir())?;
            fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
            let mut archive = legacy.clone();
            archive.generation = owner.clone();
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
            let db = Connection::open(data.join("state.sqlite3"))?;
            db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;")?;
            db.execute(
                "INSERT INTO app_state VALUES(1,?1)",
                [serde_json::to_string(&archive)?],
            )?;
            for entry in fs::read_dir(paths.history_dir())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type()?.is_file()
                    && ids.iter().any(|sid| name.starts_with(&format!("{sid}-")))
                {
                    let destination = archive_paths.history_dir().join(entry.file_name());
                    fs::copy(entry.path(), &destination)?;
                    fs::File::open(destination)?.sync_all()?;
                }
            }
            let record = Generation {
                id: owner,
                data,
                runtime: archive_paths.runtime,
                version: legacy.daemon_version.clone().unwrap_or_default(),
                build: "legacy import".into(),
                protocol: PROTOCOL_VERSION,
                catalog: CATALOG_VERSION,
                status: Status::Retired,
                pid: None,
            };
            conn.execute(
                "INSERT INTO generations VALUES(?1,?2)",
                params![record.id, serde_json::to_string(&record)?],
            )?;
        }
        drop(conn);
        fs::File::open(&temporary)?.sync_all()?;
        fs::rename(&temporary, paths.data.join("catalog.sqlite3"))?;
        fs::File::open(&paths.data)?.sync_all()?;
        Ok(())
    }

    pub fn generations(&self) -> Result<Vec<Generation>> {
        let mut query = self
            .0
            .prepare("SELECT json FROM generations ORDER BY rowid")?;
        let rows = query.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn register(&self, owner: &Generation) -> Result<()> {
        ensure!(
            owner.protocol == PROTOCOL_VERSION && owner.catalog == CATALOG_VERSION,
            "Incompatible generation"
        );
        self.0.execute(
            "INSERT INTO generations VALUES(?1,?2)",
            params![owner.id, serde_json::to_string(owner)?],
        )?;
        Ok(())
    }
    pub fn set_pid(&self, owner: &str, pid: u32) -> Result<()> {
        let mut g = self
            .generations()?
            .into_iter()
            .find(|g| g.id == owner)
            .context("Unknown owner")?;
        ensure!(
            g.status == Status::Prepared && g.pid.is_none(),
            "Owner has already started"
        );
        g.pid = Some(pid);
        self.0.execute(
            "UPDATE generations SET json=?2 WHERE id=?1",
            params![owner, serde_json::to_string(&g)?],
        )?;
        Ok(())
    }
    /// Caller holds coordination and has proved that its candidate never spawned
    /// or that its Child exited. An admitted generation must never be discarded.
    pub fn discard_prepared(&self, owner: &str, pid: Option<u32>) -> Result<bool> {
        let Some(generation) = self.generations()?.into_iter().find(|g| g.id == owner) else {
            return Ok(false);
        };
        if generation.status != Status::Prepared
            || generation.pid != pid
            || self.active()?.as_deref() == Some(owner)
        {
            return Ok(false);
        }
        self.0
            .execute("DELETE FROM generations WHERE id=?1", [owner])?;
        Ok(true)
    }
    pub fn active(&self) -> Result<Option<String>> {
        Ok(self
            .0
            .query_row("SELECT active FROM control WHERE id=1", [], |r| r.get(0))?)
    }
    fn frozen(&self) -> Result<bool> {
        let (frozen, pid): (bool, Option<u32>) = self.0.query_row(
            "SELECT frozen,freeze_pid FROM control WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if frozen
            && let Some(pid) = pid
            && unsafe { libc::kill(pid as i32, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            self.0
                .execute("UPDATE control SET frozen=0,freeze_pid=NULL WHERE id=1", [])?;
            return Ok(false);
        }
        Ok(frozen)
    }
    pub fn admitted(&self, owner: &str) -> Result<bool> {
        Ok(!self.frozen()? && self.active()?.as_deref() == Some(owner))
    }
    pub fn freeze(&self, frozen: bool) -> Result<()> {
        if frozen {
            ensure!(!self.frozen()?, "Another recovery is already in progress");
        }
        self.0.execute(
            "UPDATE control SET frozen=?1,freeze_pid=?2 WHERE id=1",
            params![frozen, frozen.then_some(std::process::id())],
        )?;
        Ok(())
    }
    /// Called after authenticated candidate verification, while coordinated.
    pub fn activate(&mut self, owner: &str) -> Result<()> {
        ensure!(!self.frozen()?, "Session creation is frozen for recovery");
        let mut owners = self.generations()?;
        ensure!(
            owners
                .iter()
                .any(|g| g.id == owner && g.status == Status::Prepared),
            "Candidate is not prepared"
        );
        ensure!(
            owners
                .iter()
                .filter(|g| g.status != Status::Retired)
                .all(|g| g.protocol == PROTOCOL_VERSION && g.catalog == CATALOG_VERSION),
            "Live owner is incompatible"
        );
        let tx = self.0.transaction()?;
        for g in &mut owners {
            if g.id == owner {
                g.status = Status::Active;
            } else if g.status == Status::Active {
                g.status = Status::Draining;
            }
            tx.execute(
                "UPDATE generations SET json=?2 WHERE id=?1",
                params![g.id, serde_json::to_string(g)?],
            )?;
        }
        tx.execute(
            "UPDATE control SET active=?1 WHERE id=1 AND frozen=0",
            [owner],
        )?;
        ensure!(tx.changes() == 1, "Session creation is frozen for recovery");
        tx.commit()?;
        Ok(())
    }
    pub fn retire(&self, owner: &str) -> Result<()> {
        let mut g = self
            .generations()?
            .into_iter()
            .find(|g| g.id == owner)
            .context("Unknown owner")?;
        g.status = Status::Retired;
        self.0.execute(
            "UPDATE generations SET json=?2 WHERE id=?1",
            params![owner, serde_json::to_string(&g)?],
        )?;
        Ok(())
    }

    /// Catalog-active + Retired is a stuck serving owner, not history.
    /// Idle shutdown still retires first; skip this once `shutdown` is set.
    pub fn restore_serving(&self) -> Result<bool> {
        let Some(active) = self.active()? else {
            return Ok(false);
        };
        let mut g = self
            .generations()?
            .into_iter()
            .find(|g| g.id == active)
            .context("Unknown active owner")?;
        if g.status != Status::Retired {
            return Ok(false);
        }
        g.status = Status::Active;
        self.0.execute(
            "UPDATE generations SET json=?2 WHERE id=?1",
            params![active, serde_json::to_string(&g)?],
        )?;
        Ok(true)
    }
    pub fn refresh(&self, state: &mut State) -> Result<()> {
        let json: String = self
            .0
            .query_row("SELECT json FROM workspace WHERE id=1", [], |r| r.get(0))?;
        let shared: State = serde_json::from_str(&json)?;
        state.projects = shared.projects;
        state.worktrees = shared.worktrees;
        state.settings = shared.settings;
        state.selected_project = shared.selected_project;
        state.catalog_revision = u64::try_from(self.0.query_row(
            "SELECT revision FROM workspace WHERE id=1",
            [],
            |r| r.get::<_, i64>(0),
        )?)?;
        Ok(())
    }
    pub fn save_workspace(&self, state: &State) -> Result<()> {
        ensure!(
            self.active()?.as_deref() == Some(&state.generation),
            "Only the active generation may save workspace data"
        );
        let mut shared = state.clone();
        clear_owned(&mut shared);
        self.0.execute(
            "UPDATE workspace SET json=?1,revision=revision+1 WHERE id=1",
            [serde_json::to_string(&shared)?],
        )?;
        Ok(())
    }
}

/// The caller holds the exclusive legacy lock. Original database and history are
/// retained; a consistent SQLite backup precedes the older-binary version guard.
pub fn migrate_idle(paths: &Paths) -> Result<()> {
    migrate(paths, false)
}

/// A legacy daemon holds this lock for its entire lifetime. Acquiring it proves
/// that service has exited, even when its last persisted records still say live.
/// Reconcile records only; never signal, adopt or rerun their recorded PIDs.
pub fn migrate_after_legacy_exit(paths: &Paths) -> Result<()> {
    let legacy = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(paths.runtime.join("daemon.lock"))?;
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

pub fn clear_owned(state: &mut State) {
    state.sessions.clear();
    state.agents.clear();
    state.notifications.clear();
    state.terminal_notices.clear();
    state.recent_events.clear();
    state.generations.clear();
}

/// Read historical records only; a failed connection never changes their lifecycle.
pub fn saved(paths: &Paths) -> Result<State> {
    let conn = Connection::open_with_flags(
        paths.data.join("state.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let json: String = conn.query_row("SELECT json FROM app_state WHERE id=1", [], |r| r.get(0))?;
    Ok(serde_json::from_str(&json)?)
}

/// Lock release plus an absent recorded process proves death. PID reuse is
/// conservatively treated as unavailable rather than declaring ownership lost.
pub fn historical(owner: &Generation, active: Option<&str>) -> bool {
    owner.status == Status::Retired && active != Some(owner.id.as_str())
}

pub fn recover_exited(root: &Paths, owner: &Generation) -> Result<bool> {
    let Some(pid) = owner.pid else {
        return Ok(false);
    };
    let _coordination = coordinate(root)?;
    if Catalog::open(root)?.active()?.as_deref() == Some(owner.id.as_str())
        && std::os::unix::net::UnixStream::connect(owner.paths().socket()).is_ok()
    {
        return Ok(false);
    }
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(owner.runtime.join("daemon.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(false);
    }
    if unsafe { libc::kill(pid as i32, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return Ok(false);
    }
    let mut state = saved(&owner.paths())?;
    for session in &mut state.sessions {
        if session.lifecycle.live() {
            session.lifecycle = Lifecycle::Interrupted;
            session.pid = None;
        }
    }
    for agent in &mut state.agents {
        if !matches!(
            agent.state,
            AgentState::Completed | AgentState::Failed | AgentState::Stopped
        ) {
            agent.state = AgentState::Unknown;
        }
    }
    state.revision += 1;
    let conn = Connection::open(owner.data.join("state.sqlite3"))?;
    conn.execute(
        "UPDATE app_state SET json=?1 WHERE id=1",
        [serde_json::to_string(&state)?],
    )?;
    Catalog::open(root)?.retire(&owner.id)?;
    // The exclusive owner lock and dead PID permit cleanup of this generation
    // only. Its database and history remain available to the active service.
    for entry in fs::read_dir(&owner.data)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir()
            && (name == "bin" || name.to_string_lossy().starts_with(".daemon-helper-"))
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let _ = fs::remove_file(owner.paths().socket());
    let _ = fs::remove_file(owner.paths().auth());
    Ok(true)
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

pub fn snapshot(paths: &Paths) -> Result<State> {
    let catalog = Catalog::open(paths)?;
    let active = catalog.active()?.context("No active generation")?;
    let mut aggregate = State::default();
    let mut inventories = Vec::new();
    for owner in catalog.generations()? {
        if owner.status == Status::Prepared {
            continue;
        }
        let (state, error) = if historical(&owner, Some(active.as_str())) {
            (saved(&owner.paths())?, None)
        } else {
            match crate::rpc(&owner.paths(), Request::Snapshot) {
                Ok(Response::State(s)) => (*s, None),
                result => (
                    saved(&owner.paths())?,
                    Some(format!("Owner unavailable: {result:?}")),
                ),
            }
        };
        if owner.id == active {
            aggregate = state.clone();
        }
        inventories.push((owner, state, error));
    }
    clear_owned(&mut aggregate);
    aggregate.generation = active;
    catalog.refresh(&mut aggregate)?;
    for (owner, state, error) in inventories {
        aggregate.generations.push(Health {
            live_sessions: state.sessions.iter().filter(|s| s.lifecycle.live()).count(),
            owner,
            revision: state.revision,
            error,
            capabilities: state.capabilities,
            helper: state.attachment_helper_executable,
        });
        aggregate.sessions.extend(state.sessions);
        aggregate.agents.extend(state.agents);
        aggregate.notifications.extend(state.notifications);
        aggregate.terminal_notices.extend(state.terminal_notices);
    }
    Ok(aggregate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_catalog_cannot_bind_a_different_data_directory() {
        let (_directory, root, catalog) = fixture();
        let registered = owner(&root, &catalog);
        assert!(validate_endpoint(&root, &registered.paths(), &registered.id).is_ok());
        let other = tempfile::tempdir().unwrap();
        let paths = Paths::at(other.path().into());
        paths.init().unwrap();
        assert!(
            validate_endpoint(&root, &paths, &registered.id)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
        assert!(validate_endpoint(&root, &registered.paths(), "unknown").is_err());
    }

    #[test]
    fn legacy_exit_recovery_requires_lock_and_preserves_original_records() {
        let dir = tempfile::Builder::new()
            .prefix("legacy-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let mut old = session("legacy-owner", Lifecycle::Running);
        // A recorded PID must never be signalled or adopted by migration.
        old.pid = Some(std::process::id());
        let state = State {
            sessions: vec![old.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        let lock = fs::File::create(paths.runtime.join("daemon.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(migrate_after_legacy_exit(&paths).is_err());
        assert!(!exists(&paths));
        assert!(saved(&paths).unwrap().sessions[0].lifecycle.live());
        drop(lock);
        migrate_after_legacy_exit(&paths).unwrap();
        let owners = Catalog::open(&paths).unwrap().generations().unwrap();
        assert_eq!(owners.len(), 1);
        let imported = saved(&owners[0].paths()).unwrap();
        assert_eq!(imported.sessions[0].id, old.id);
        assert_eq!(imported.sessions[0].generation, old.generation);
        assert_eq!(imported.sessions[0].lifecycle, Lifecycle::Interrupted);
        assert_eq!(imported.sessions[0].pid, None);
        assert!(saved(&paths).unwrap().sessions[0].lifecycle.live());
        let backup = Connection::open(paths.data.join("state.before-generations.sqlite3")).unwrap();
        let json: String = backup
            .query_row("SELECT json FROM app_state WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert!(
            serde_json::from_str::<State>(&json).unwrap().sessions[0]
                .lifecycle
                .live()
        );
        assert_eq!(unsafe { libc::kill(std::process::id() as i32, 0) }, 0);
    }

    #[test]
    fn discarding_a_failed_candidate_cannot_remove_an_active_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let active = owner(&paths, &catalog);
        catalog.activate(&active.id).unwrap();
        let failed = owner(&paths, &catalog);
        assert!(catalog.discard_prepared(&failed.id, None).unwrap());
        assert!(!catalog.discard_prepared(&active.id, None).unwrap());
        let exited = owner(&paths, &catalog);
        catalog.set_pid(&exited.id, 123).unwrap();
        assert!(!catalog.discard_prepared(&exited.id, Some(456)).unwrap());
        assert!(catalog.discard_prepared(&exited.id, Some(123)).unwrap());
        assert_eq!(catalog.generations().unwrap().len(), 1);
        assert!(catalog.admitted(&active.id).unwrap());
    }

    fn fixture() -> (tempfile::TempDir, Paths, Catalog) {
        let dir = tempfile::Builder::new()
            .prefix("gen-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        migrate_idle(&paths).unwrap();
        let catalog = Catalog::open(&paths).unwrap();
        (dir, paths, catalog)
    }
    fn owner(paths: &Paths, catalog: &Catalog) -> Generation {
        let id = id();
        let g = Generation {
            data: paths.data.join("generations").join(&id),
            runtime: paths.runtime.join(&id[..8]),
            id,
            version: "1.0.0".into(),
            build: "fixture".into(),
            protocol: PROTOCOL_VERSION,
            catalog: CATALOG_VERSION,
            status: Status::Prepared,
            pid: None,
        };
        g.paths().init().unwrap();
        let db = Connection::open(g.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&State {
                generation: g.id.clone(),
                ..Default::default()
            })
            .unwrap()],
        )
        .unwrap();
        catalog.register(&g).unwrap();
        g
    }
    fn session(generation: &str, lifecycle: Lifecycle) -> Session {
        Session {
            id: id(),
            project_id: id(),
            label: "fixture".into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle,
            created: now(),
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: generation.into(),
            pid: None,
            truncated: false,
            cwd_confirmed: false,
            review: false,
        }
    }
    fn save(g: &Generation, state: &State) {
        Connection::open(g.data.join("state.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE app_state SET json=?1",
                [serde_json::to_string(state).unwrap()],
            )
            .unwrap();
    }

    #[test]
    fn activation_preserves_old_session_identity_and_routes_new_work() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let session = session(&a.id, Lifecycle::Running);
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session.clone()],
                ..Default::default()
            },
        );
        catalog.activate(&b.id).unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
        assert!(catalog.admitted(&b.id).unwrap());
        assert_eq!(
            owner_for(
                &paths,
                &Request::Stop {
                    session: session.id.clone()
                }
            )
            .unwrap()
            .data,
            a.data
        );
        assert_eq!(
            owner_for(
                &paths,
                &Request::AddProject {
                    path: "/tmp".into()
                }
            )
            .unwrap()
            .data,
            b.data
        );
        let preserved = saved(&a.paths()).unwrap();
        assert_eq!(preserved.sessions[0].id, session.id);
        assert!(preserved.sessions[0].lifecycle.live());
    }

    #[test]
    fn draining_owner_cannot_overwrite_shared_workspace() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let mut state = State {
            generation: a.id.clone(),
            ..Default::default()
        };
        state.projects.push(Project {
            id: id(),
            name: "keep".into(),
            path: "/tmp".into(),
            layout: serde_json::json!({"version":999}),
        });
        catalog.save_workspace(&state).unwrap();
        catalog.activate(&b.id).unwrap();
        state.projects.clear();
        assert!(catalog.save_workspace(&state).is_err());
        catalog.refresh(&mut state).unwrap();
        assert_eq!(state.projects[0].layout["version"], 999);
    }

    #[test]
    fn frozen_activation_rolls_back_all_generation_statuses() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.freeze(true).unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
        assert!(catalog.activate(&b.id).is_err());
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Active);
        catalog.freeze(false).unwrap();
        assert!(catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn unavailable_owner_keeps_live_ownership_and_blocks_death_inference() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.set_pid(&a.id, std::process::id()).unwrap();
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session(&a.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        let lock = fs::File::create(a.runtime.join("daemon.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        let registered = catalog.generations().unwrap().remove(0);
        assert!(!recover_exited(&paths, &registered).unwrap());
        let inventory = snapshot(&paths).unwrap();
        assert!(inventory.sessions[0].lifecycle.live());
        assert!(inventory.generations[0].error.is_some());
        drop(lock);
        assert!(
            !recover_exited(&paths, &registered).unwrap(),
            "A live process still owns the identity even without the lock"
        );
    }

    #[test]
    fn empty_catalog_file_does_not_block_migration() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        fs::write(paths.data.join("catalog.sqlite3"), []).unwrap();
        assert!(!exists(&paths));
        migrate_idle(&paths).unwrap();
        assert_eq!(catalog_version(&paths), Some(CATALOG_VERSION));
        Catalog::open(&paths).unwrap();
    }

    #[test]
    fn migration_rejects_live_sessions_without_publishing_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let legacy = State {
            sessions: vec![session("legacy", Lifecycle::Running)],
            ..Default::default()
        };
        assert!(Catalog::import(&paths, &legacy).is_err());
        assert!(!exists(&paths));
    }

    #[test]
    fn migration_keeps_session_ids_scrollback_and_original_database() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let session = session("legacy", Lifecycle::Ended);
        let state = State {
            sessions: vec![session.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        let history = format!("{}-00000000000000000001.pty", session.id);
        fs::write(paths.history_dir().join(&history), b"retained").unwrap();
        migrate_idle(&paths).unwrap();
        let archive = Catalog::open(&paths)
            .unwrap()
            .generations()
            .unwrap()
            .remove(0);
        assert_eq!(saved(&archive.paths()).unwrap().sessions[0].id, session.id);
        assert_eq!(
            fs::read(archive.paths().history_dir().join(&history)).unwrap(),
            b"retained"
        );
        assert_eq!(
            fs::read(paths.history_dir().join(&history)).unwrap(),
            b"retained"
        );
        assert!(
            paths
                .data
                .join("state.before-generations.sqlite3")
                .is_file()
        );
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn coordination_serializes_activation_against_admission() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let guard = coordinate(&paths).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let other = paths.clone();
        let b_id = b.id.clone();
        let worker = std::thread::spawn(move || {
            let _guard = coordinate(&other).unwrap();
            Catalog::open(&other).unwrap().activate(&b_id).unwrap();
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert!(catalog.admitted(&a.id).unwrap());
        drop(guard);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn uncertain_creation_response_is_never_retried() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = UnixListener::bind(a.paths().socket()).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(envelope.request, Request::Create { .. }));
            // Model a spawn whose response is lost. The client cannot infer that
            // execution failed, and must never send a second creation.
            drop(stream);
            listener
        });
        let request = Request::Create {
            project: id(),
            cwd: None,
            file: None,
            line: None,
            column: None,
            editor: false,
        };
        assert!(rpc(&paths, request).is_err());
        let listener = server.join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err());
    }

    #[test]
    fn only_explicit_pre_spawn_redirect_retries_on_active_owner() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        for g in [&a, &b] {
            atomic_write(&g.paths().auth(), b"fixture").unwrap();
        }
        let first = UnixListener::bind(a.paths().socket()).unwrap();
        let second = UnixListener::bind(b.paths().socket()).unwrap();
        let root = paths.clone();
        let new_id = b.id.clone();
        let created = session(&b.id, Lifecycle::Running);
        let expected = created.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = first.accept().unwrap();
            let _: Envelope = read_frame(&mut stream).unwrap();
            {
                let _guard = coordinate(&root).unwrap();
                Catalog::open(&root).unwrap().activate(&new_id).unwrap();
            }
            write_frame(&mut stream, &Response::Redirect { generation: new_id }).unwrap();
            let (mut stream, _) = second.accept().unwrap();
            let _: Envelope = read_frame(&mut stream).unwrap();
            write_frame(&mut stream, &Response::Created(created)).unwrap();
        });
        let Response::Created(actual) = rpc(
            &paths,
            Request::Create {
                project: id(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
        )
        .unwrap() else {
            panic!("created");
        };
        assert_eq!(actual.id, expected);
        assert_eq!(actual.generation, b.id);
        server.join().unwrap();
    }

    #[test]
    fn interrupted_import_keeps_backup_and_retries_without_duplicate_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let historical = session("legacy", Lifecycle::Ended);
        let state = State {
            sessions: vec![historical.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        fs::remove_dir(paths.history_dir()).unwrap();
        assert!(migrate_idle(&paths).is_err());
        assert!(!exists(&paths));
        assert!(
            paths
                .data
                .join("state.before-generations.sqlite3")
                .is_file()
        );
        assert_eq!(saved(&paths).unwrap().sessions[0].id, historical.id);
        fs::create_dir(paths.history_dir()).unwrap();
        migrate_idle(&paths).unwrap();
        let owners = Catalog::open(&paths).unwrap().generations().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(
            saved(&owners[0].paths()).unwrap().sessions[0].id,
            historical.id
        );
    }

    #[test]
    fn incompatible_candidate_and_second_recovery_preserve_current_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let mut incompatible = a.clone();
        incompatible.id = id();
        incompatible.protocol += 1;
        assert!(catalog.register(&incompatible).is_err());
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        catalog.freeze(true).unwrap();
        assert!(catalog.freeze(true).is_err());
        catalog.freeze(false).unwrap();
        assert!(catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn restore_serving_repairs_catalog_active_marked_retired() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.retire(&a.id).unwrap();
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Retired);
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        assert!(catalog.restore_serving().unwrap());
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Active);
        assert!(catalog.admitted(&a.id).unwrap());
        assert!(!catalog.restore_serving().unwrap());
    }

    #[test]
    fn create_reaches_catalog_active_owner_marked_retired() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.retire(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = UnixListener::bind(a.paths().socket()).unwrap();
        let generation = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(
                matches!(envelope.request, Request::Create { .. }),
                "Create must not be wrapped as Archived"
            );
            write_frame(
                &mut stream,
                &Response::Created(session(&generation, Lifecycle::Running)),
            )
            .unwrap();
        });
        let Response::Created(_) = rpc(
            &paths,
            Request::Create {
                project: id(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
        )
        .unwrap() else {
            panic!("created");
        };
        server.join().unwrap();
    }

    #[test]
    fn historical_owner_requests_stay_archived() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let record = session(&a.id, Lifecycle::Ended);
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![record.clone()],
                ..Default::default()
            },
        );
        catalog.activate(&b.id).unwrap();
        catalog.retire(&a.id).unwrap();
        atomic_write(&b.paths().auth(), b"fixture").unwrap();
        let listener = UnixListener::bind(b.paths().socket()).unwrap();
        let expected = record.id.clone();
        let archived = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            match envelope.request {
                Request::Archived {
                    generation,
                    request,
                } => {
                    assert_eq!(generation, archived);
                    assert!(matches!(
                        *request,
                        Request::History { session } if session == expected
                    ));
                }
                other => panic!("{other:?}"),
            }
            write_frame(&mut stream, &Response::Ok).unwrap();
        });
        rpc(
            &paths,
            Request::History {
                session: record.id.clone(),
            },
        )
        .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn snapshot_contacts_catalog_active_owner_marked_retired() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                daemon_version: Some("from-disk".into()),
                ..Default::default()
            },
        );
        catalog.retire(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = UnixListener::bind(a.paths().socket()).unwrap();
        let generation = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(envelope.request, Request::Snapshot));
            snapshot::write_response(
                &mut stream,
                &Response::State(Box::new(State {
                    generation,
                    daemon_version: Some("from-socket".into()),
                    ..Default::default()
                })),
                envelope.snapshot_chunks,
            )
            .unwrap();
        });
        let state = snapshot(&paths).unwrap();
        assert_eq!(state.daemon_version.as_deref(), Some("from-socket"));
        server.join().unwrap();
    }

    #[test]
    fn recover_exited_does_not_retire_a_listening_catalog_active_owner() {
        use std::os::unix::net::UnixListener;
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        catalog.set_pid(&a.id, pid).unwrap();
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session(&a.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        let _listener = UnixListener::bind(a.paths().socket()).unwrap();
        let registered = catalog.generations().unwrap().remove(0);
        assert!(!recover_exited(&paths, &registered).unwrap());
        assert_eq!(
            Catalog::open(&paths).unwrap().generations().unwrap()[0].status,
            Status::Active
        );
        assert!(saved(&a.paths()).unwrap().sessions[0].lifecycle.live());
    }
}
