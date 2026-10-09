use crate::{
    BTreeSet, Context, Deserialize, Duration, PROTOCOL_VERSION, Path, PathBuf, Paths, Read, Result,
    Serialize, State, ensure, exe_name, fs, id,
};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, params};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use super::recovery::clear_owned;

pub const CAPABILITY: &str = "daemon-generations-v1";
pub const CATALOG_VERSION: u32 = 1;

/// Diagnostic identity for an immutable executable set. Verification additionally
/// compares complete files; this fingerprint is not a cryptographic trust check.
pub fn build_identity(bin: &Path) -> Result<String> {
    use std::hash::Hasher;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for name in [exe_name("terminator-daemon"), exe_name("terminator-hook")] {
        let mut file = fs::File::open(bin.join(name))?;
        hash.write_u64(file.metadata()?.len());
        let mut bytes = [0; 65536];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            if let Some(chunk) = bytes.get(..count) {
                hash.write(chunk);
            }
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
    #[must_use]
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

#[must_use]
pub fn exists(paths: &Paths) -> bool {
    catalog_version(paths).is_some_and(|version| version > 0)
}

pub(super) fn catalog_version(paths: &Paths) -> Option<u32> {
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
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(paths.data.join("coordination.lock"))?;
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
            #[cfg(unix)]
            fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
            let mut archive = legacy.clone();
            archive.generation.clone_from(&owner);
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
                    fs::OpenOptions::new()
                        .write(true)
                        .open(destination)?
                        .sync_all()?;
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
        fs::OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        fs::rename(&temporary, paths.data.join("catalog.sqlite3"))?;
        // Windows does not support opening a directory with File::open.
        #[cfg(unix)]
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
    /// Shared workspace revision without re-reading and decoding the workspace
    /// JSON. Used to detect no-op conditional snapshots cheaply.
    pub fn revision(&self) -> Result<u64> {
        Ok(u64::try_from(self.0.query_row(
            "SELECT revision FROM workspace WHERE id=1",
            [],
            |r| r.get::<_, i64>(0),
        )?)?)
    }
    fn frozen(&self) -> Result<bool> {
        let (frozen, pid): (bool, Option<u32>) = self.0.query_row(
            "SELECT frozen,freeze_pid FROM control WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if frozen
            && let Some(pid) = pid
            && crate::signals::process_gone(pid)
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
    pub fn remove(&self, owner: &str) -> Result<()> {
        self.0
            .execute("DELETE FROM generations WHERE id=?1", [owner])?;
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
