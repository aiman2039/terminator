use anyhow::{Context, Result, anyhow, bail, ensure};
use rusqlite::{Connection, params};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use terminator_core::*;
pub struct Store {
    conn: Connection,
    owner: Option<String>,
}
impl Store {
    pub fn open(paths: &Paths) -> Result<(Self, State)> {
        // Probe the version first: a newer database must fail loudly and
        // must never be clobbered by the quarantine path below. An
        // unreadable probe (missing or garbage file) falls through to a
        // fresh or quarantined open.
        if let Ok(conn) = Connection::open(paths.data.join("state.sqlite3")) {
            let version: u32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap_or(0);
            ensure!(version <= 1, "Database is newer than this application");
        }
        match Self::read(paths) {
            Ok(opened) => Ok(opened),
            Err(error) => {
                // A damaged store must never prevent startup: quarantine the
                // files for forensics and start from defaults. If the
                // directory itself is unusable the fresh open fails too and
                // that error propagates, as before.
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                for suffix in ["", "-wal", "-shm", "-journal"] {
                    let from = paths.data.join(format!("state.sqlite3{suffix}"));
                    if from.exists() {
                        let _ = fs::rename(
                            &from,
                            paths
                                .data
                                .join(format!("state.corrupt-{stamp}.sqlite3{suffix}")),
                        );
                    }
                }
                let (store, mut state) = Self::read(paths).context(
                    "Saved state was unreadable and quarantine did not produce a fresh store",
                )?;
                state.degraded = Some(format!(
                    "Saved state was unreadable and was quarantined; started with defaults: {error:#}"
                ));
                Ok((store, state))
            }
        }
    }
    fn read(paths: &Paths) -> Result<(Self, State)> {
        let conn = Connection::open(paths.data.join("state.sqlite3"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=3000;",
        )?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS app_state (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL); PRAGMA user_version=1;")?;
        let mut state: State =
            match conn.query_row::<String, _, _>("SELECT json FROM app_state WHERE id=1", [], |r| {
                r.get(0)
            }) {
                Ok(json) => {
                    serde_json::from_str(&json).context("Stored state is not valid JSON")?
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => State::default(),
                Err(e) => return Err(e.into()),
            };
        if state.settings.validate().is_err() {
            // Persisted settings that fail current validation could never
            // have been saved by this build (saves are validated), so they
            // come from corruption or hand edits. Reset rather than running
            // with values no code path accepts.
            state.settings = Settings::default();
            state.degraded =
                Some("Stored settings failed validation and were reset to defaults.".into());
        }
        // Presence is live-only; nothing persisted may claim it.
        state.presence.clear();
        Ok((Self { conn, owner: None }, state))
    }
    pub fn bind_owner(&mut self, owner: String) {
        self.owner = Some(owner);
    }
    pub fn save(&self, state: &State) -> Result<()> {
        let mut owned;
        let mut stripped;
        let state = if let Some(owner) = &self.owner {
            ensure!(&state.generation == owner, "Session store owner changed");
            ensure!(
                state.sessions.iter().all(|s| &s.generation == owner),
                "Foreign session in owner store"
            );
            owned = state.clone();
            owned.projects.clear();
            owned.worktrees.clear();
            owned.vnc_profiles.clear();
            owned.selected_project = None;
            owned.generations.clear();
            owned.presence.clear();
            &owned
        } else if state.presence.is_empty() {
            state
        } else {
            // Presence is live-only and never persisted; no migration needed.
            stripped = state.clone();
            stripped.presence.clear();
            &stripped
        };
        self.conn.execute("INSERT INTO app_state(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json WHERE json_extract(excluded.json,'$.revision') >= json_extract(app_state.json,'$.revision')",params![serde_json::to_string(state)?])?;
        Ok(())
    }
}
pub fn history_files(paths: &Paths, session: Option<&str>) -> Vec<PathBuf> {
    let mut files = fs::read_dir(paths.history_dir())
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|s| s == "pty")
                && session.is_none_or(|s| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with(&format!("{s}-"))
                })
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}
struct Segment {
    session: String,
    size: u64,
    created: u64,
}
struct Writer {
    file: std::io::BufWriter<fs::File>,
    path: PathBuf,
    opened: std::time::Instant,
}
/// All retained-file mutations are owned by the history worker.
pub struct History {
    paths: Paths,
    writers: std::collections::HashMap<String, Writer>,
    segments: std::collections::BTreeMap<PathBuf, Segment>,
    removed: std::collections::HashSet<String>,
    total: u64,
    sequence: u128,
    per_session: std::collections::HashMap<String, u64>,
}
impl History {
    pub fn new(paths: Paths) -> Result<Self> {
        let mut history = Self {
            paths,
            writers: std::collections::HashMap::default(),
            segments: std::collections::BTreeMap::default(),
            removed: std::collections::HashSet::default(),
            total: 0,
            sequence: 0,
            per_session: std::collections::HashMap::default(),
        };
        for path in history_files(&history.paths, None) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let Some(session) = name.get(..36).filter(|s| uuid::Uuid::parse_str(s).is_ok()) else {
                continue;
            };
            let metadata = fs::metadata(&path)?;
            let created = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let segment = Segment {
                session: session.into(),
                size: metadata.len(),
                created,
            };
            history.total = history.total.saturating_add(segment.size);
            let size = segment.size;
            let usage = history
                .per_session
                .entry(segment.session.clone())
                .or_default();
            *usage = usage.saturating_add(size);
            history.segments.insert(path, segment);
        }
        Ok(history)
    }
    pub fn append(&mut self, session: &str, mut data: &[u8]) -> Result<()> {
        if self.removed.contains(session) {
            return Ok(());
        }
        while !data.is_empty() {
            data = self.append_chunk(session, data)?;
        }
        Ok(())
    }
    fn append_chunk<'a>(&mut self, session: &str, data: &'a [u8]) -> Result<&'a [u8]> {
        const LIMIT: u64 = 4 * 1024 * 1024;
        self.rotate_writer_if_needed(session, LIMIT)?;
        if !self.writers.contains_key(session) {
            self.open_writer(session)?;
        }
        let path = self
            .writers
            .get(session)
            .ok_or_else(|| anyhow!("Missing history writer"))?
            .path
            .clone();
        let room = LIMIT.saturating_sub(
            self.segments
                .get(&path)
                .ok_or_else(|| anyhow!("Missing history segment"))?
                .size,
        );
        let Ok(room) = usize::try_from(room) else {
            bail!("History segment does not fit");
        };
        let n = data.len().min(room);
        if n == 0 {
            bail!("History segment is full");
        }
        let chunk = data
            .get(..n)
            .ok_or_else(|| anyhow!("History write is out of range"))?;
        self.writers
            .get_mut(session)
            .ok_or_else(|| anyhow!("Missing history writer"))?
            .file
            .write_all(chunk)?;
        let bytes = u64::try_from(n).map_err(|_| anyhow!("History write does not fit"))?;
        let segment = self
            .segments
            .get_mut(&path)
            .ok_or_else(|| anyhow!("Missing history segment"))?;
        segment.size = segment.size.saturating_add(bytes);
        self.total = self.total.saturating_add(bytes);
        let usage = self.per_session.entry(session.into()).or_default();
        *usage = usage.saturating_add(bytes);
        data.get(n..)
            .ok_or_else(|| anyhow!("History write is out of range"))
    }
    fn rotate_writer_if_needed(&mut self, session: &str, limit: u64) -> Result<()> {
        let rotate = if let Some(writer) = self.writers.get(session) {
            let size = self
                .segments
                .get(&writer.path)
                .map(|segment| segment.size)
                .ok_or_else(|| anyhow!("Missing history segment"))?;
            size >= limit || writer.opened.elapsed() >= std::time::Duration::from_mins(5)
        } else {
            false
        };
        if rotate {
            self.writers
                .remove(session)
                .ok_or_else(|| anyhow!("Missing history writer"))?
                .file
                .flush()?;
        }
        Ok(())
    }
    fn open_writer(&mut self, session: &str) -> Result<()> {
        self.sequence = self.sequence.saturating_add(1).max(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        let path = self.paths.history_dir().join(format!(
            "{session}-{:020}_{:030}.pty",
            now(),
            self.sequence
        ));
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        self.segments.insert(
            path.clone(),
            Segment {
                session: session.into(),
                size: 0,
                created: now(),
            },
        );
        self.writers.insert(
            session.into(),
            Writer {
                file: std::io::BufWriter::with_capacity(65536, file),
                path,
                opened: std::time::Instant::now(),
            },
        );
        Ok(())
    }
    pub fn flush(&mut self) -> Result<()> {
        for writer in self.writers.values_mut() {
            writer.file.flush()?;
        }
        Ok(())
    }
    pub fn exceeds(&self, settings: &Settings) -> bool {
        let Some(total_limit) = settings
            .total_mib
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
        else {
            return true;
        };
        let Some(session_limit) = settings
            .session_mib
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
        else {
            return true;
        };
        self.total > total_limit || self.per_session.values().any(|n| *n > session_limit)
    }
    fn delete(&mut self, path: &PathBuf) -> Result<String> {
        let session = self
            .segments
            .get(path)
            .ok_or_else(|| anyhow!("Missing history segment"))?
            .session
            .clone();
        if self.writers.get(&session).is_some_and(|w| &w.path == path) {
            self.writers
                .remove(&session)
                .ok_or_else(|| anyhow!("Missing history writer"))?
                .file
                .flush()?;
        }
        fs::remove_file(path)?;
        let segment = self
            .segments
            .remove(path)
            .ok_or_else(|| anyhow!("Missing history segment"))?;
        self.total = self.total.saturating_sub(segment.size);
        if let Some(size) = self.per_session.get_mut(&session) {
            *size = size.saturating_sub(segment.size);
        }
        Ok(session)
    }
    pub fn prune(&mut self, settings: &Settings) -> Result<Vec<String>> {
        let budget = settings
            .total_mib
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
            .ok_or_else(|| anyhow!("History budget overflow"))?;
        self.prune_with_budget(settings, budget)
    }
    pub fn prune_with_budget(&mut self, settings: &Settings, budget: u64) -> Result<Vec<String>> {
        let max_age = settings
            .history_days
            .checked_mul(86400)
            .ok_or_else(|| anyhow!("History age limit overflows"))?;
        let session_limit = settings
            .session_mib
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
            .ok_or_else(|| anyhow!("History budget overflow"))?;
        let mut paths: Vec<_> = self.segments.keys().cloned().collect();
        paths.sort_by_key(|p| {
            self.segments
                .get(p)
                .map(|segment| segment.created)
                .unwrap_or(0)
        });
        let mut removed = Vec::new();
        for path in paths {
            let (created, session) = {
                let segment = self
                    .segments
                    .get(&path)
                    .ok_or_else(|| anyhow!("Missing history segment"))?;
                (segment.created, segment.session.clone())
            };
            let session_usage = self
                .per_session
                .get(&session)
                .copied()
                .ok_or_else(|| anyhow!("Missing session history"))?;
            if now().saturating_sub(created) > max_age
                || self.total > budget
                || session_usage > session_limit
            {
                removed.push(self.delete(&path)?);
            }
        }
        removed.sort();
        removed.dedup();
        Ok(removed)
    }
    pub fn clear(&mut self, session: Option<&str>, remove: bool) -> Result<()> {
        if remove && let Some(session) = session {
            self.removed.insert(session.into());
        }
        let paths: Vec<_> = self
            .segments
            .iter()
            .filter(|(_, s)| session.is_none_or(|id| id == s.session))
            .map(|(p, _)| p.clone())
            .collect();
        for path in paths {
            self.delete(&path)?;
        }
        Ok(())
    }
}
pub fn text(paths: &Paths, session: &Session) -> Result<String> {
    let mut parser = vt100::Parser::new(session.rows.max(1), session.cols.max(1), 10_000);
    let mut total: u64 = 0;
    let mut files = history_files(paths, Some(&session.id));
    files.reverse();
    let mut selected = Vec::new();
    for p in files {
        let size = fs::metadata(&p)?.len();
        let Some(next) = total.checked_add(size) else {
            break;
        };
        if next > 16 * 1024 * 1024 {
            break;
        }
        total = next;
        selected.push(p);
    }
    selected.reverse();
    for p in selected {
        let mut f = fs::File::open(p)?;
        let mut buf = [0; 8192];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let Some(chunk) = buf.get(..n) else {
                break;
            };
            parser.process(chunk);
        }
    }
    Ok(screen_text(parser.screen(), 10_000))
}
pub fn screen_text(screen: &vt100::Screen, limit: usize) -> String {
    let mut screen = screen.clone();
    screen.set_scrollback(limit);
    let max = screen.scrollback();
    let (rows, cols) = screen.size();
    let mut output = String::new();
    for offset in (1..=max).rev() {
        screen.set_scrollback(offset);
        if let Some(row) = screen.rows(0, cols).next() {
            output.push_str(&row);
            output.push('\n');
        }
    }
    screen.set_scrollback(0);
    for row in screen.rows(0, cols).take(rows as usize) {
        output.push_str(&row);
        output.push('\n');
    }
    output
}
#[cfg(test)]
mod tests;
