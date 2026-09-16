use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};
use terminator_core::*;
pub struct Store {
    conn: Connection,
    owner: Option<String>,
}
impl Store {
    pub fn open(paths: &Paths) -> Result<(Self, State)> {
        let conn = Connection::open(paths.data.join("state.sqlite3"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=3000;",
        )?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 1, "Database is newer than this application");
        conn.execute_batch("CREATE TABLE IF NOT EXISTS app_state (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL); PRAGMA user_version=1;")?;
        let state =
            match conn.query_row::<String, _, _>("SELECT json FROM app_state WHERE id=1", [], |r| {
                r.get(0)
            }) {
                Ok(json) => serde_json::from_str(&json)?,
                Err(rusqlite::Error::QueryReturnedNoRows) => State::default(),
                Err(e) => return Err(e.into()),
            };
        Ok((Self { conn, owner: None }, state))
    }
    pub fn bind_owner(&mut self, owner: String) {
        self.owner = Some(owner);
    }
    pub fn save(&self, state: &State) -> Result<()> {
        let mut owned;
        let state = if let Some(owner) = &self.owner {
            ensure!(&state.generation == owner, "Session store owner changed");
            ensure!(
                state.sessions.iter().all(|s| &s.generation == owner),
                "Foreign session in owner store"
            );
            owned = state.clone();
            owned.projects.clear();
            owned.worktrees.clear();
            owned.selected_project = None;
            owned.generations.clear();
            &owned
        } else {
            state
        };
        self.conn.execute("INSERT INTO app_state(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json WHERE json_extract(excluded.json,'$.revision') >= json_extract(app_state.json,'$.revision')",params![serde_json::to_string(state)?])?;
        Ok(())
    }
}
pub fn history_files(paths: &Paths, session: Option<&str>) -> Vec<PathBuf> {
    let mut files = fs::read_dir(paths.history_dir())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
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
            writers: Default::default(),
            segments: Default::default(),
            removed: Default::default(),
            total: 0,
            sequence: 0,
            per_session: Default::default(),
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
            history.total += segment.size;
            *history
                .per_session
                .entry(segment.session.clone())
                .or_default() += segment.size;
            history.segments.insert(path, segment);
        }
        Ok(history)
    }
    pub fn append(&mut self, session: &str, mut data: &[u8]) -> Result<()> {
        if self.removed.contains(session) {
            return Ok(());
        }
        const LIMIT: u64 = 4 * 1024 * 1024;
        while !data.is_empty() {
            if self.writers.get(session).is_some_and(|w| {
                self.segments[&w.path].size >= LIMIT
                    || w.opened.elapsed() >= std::time::Duration::from_secs(300)
            }) {
                self.writers.remove(session).unwrap().file.flush()?;
            }
            if !self.writers.contains_key(session) {
                self.sequence = (self.sequence + 1).max(
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
            }
            let writer = self.writers.get_mut(session).unwrap();
            let segment = self.segments.get_mut(&writer.path).unwrap();
            let n = data.len().min((LIMIT - segment.size) as usize);
            writer.file.write_all(&data[..n])?;
            segment.size += n as u64;
            self.total += n as u64;
            *self.per_session.entry(session.into()).or_default() += n as u64;
            data = &data[n..];
        }
        Ok(())
    }
    pub fn flush(&mut self) -> Result<()> {
        for writer in self.writers.values_mut() {
            writer.file.flush()?;
        }
        Ok(())
    }
    pub fn exceeds(&self, settings: &Settings) -> bool {
        self.total > settings.total_mib * 1024 * 1024
            || self
                .per_session
                .values()
                .any(|n| *n > settings.session_mib * 1024 * 1024)
    }
    fn delete(&mut self, path: &PathBuf) -> Result<String> {
        let session = self.segments[path].session.clone();
        if self.writers.get(&session).is_some_and(|w| &w.path == path) {
            self.writers.remove(&session).unwrap().file.flush()?;
        }
        fs::remove_file(path)?;
        let segment = self.segments.remove(path).unwrap();
        self.total = self.total.saturating_sub(segment.size);
        if let Some(size) = self.per_session.get_mut(&session) {
            *size = size.saturating_sub(segment.size);
        }
        Ok(session)
    }
    pub fn prune(&mut self, settings: &Settings) -> Result<Vec<String>> {
        self.prune_with_budget(settings, settings.total_mib * 1024 * 1024)
    }
    pub fn prune_with_budget(&mut self, settings: &Settings, budget: u64) -> Result<Vec<String>> {
        let mut paths: Vec<_> = self.segments.keys().cloned().collect();
        paths.sort_by_key(|p| self.segments[p].created);
        let mut removed = Vec::new();
        for path in paths {
            let segment = &self.segments[&path];
            if now().saturating_sub(segment.created) > settings.history_days * 86400
                || self.total > budget
                || self.per_session[&segment.session] > settings.session_mib * 1024 * 1024
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
    let mut total = 0;
    let mut files = history_files(paths, Some(&session.id));
    files.reverse();
    let mut selected = Vec::new();
    for p in files {
        let size = fs::metadata(&p)?.len();
        if total + size > 16 * 1024 * 1024 {
            break;
        }
        total += size;
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
            parser.process(&buf[..n]);
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
mod tests {
    use super::*;
    #[test]
    fn rotation_retention_clear_and_remove_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let sid = id();
        let mut history = History::new(paths.clone()).unwrap();
        history.append(&sid, &vec![b'a'; 4 * 1024 * 1024]).unwrap();
        history.append(&sid, b"tail").unwrap();
        history.flush().unwrap();
        let files = history_files(&paths, Some(&sid));
        assert_eq!(files.len(), 2);
        assert_eq!(fs::read(&files[1]).unwrap(), b"tail");
        assert_eq!(
            history
                .prune(&Settings {
                    session_mib: 1,
                    ..Default::default()
                })
                .unwrap(),
            vec![sid.clone()]
        );
        assert_eq!(history_files(&paths, Some(&sid)).len(), 1);
        history.clear(Some(&sid), false).unwrap();
        history.append(&sid, b"after clear").unwrap();
        history.flush().unwrap();
        assert_eq!(
            fs::read(&history_files(&paths, Some(&sid))[0]).unwrap(),
            b"after clear"
        );
        history.clear(Some(&sid), true).unwrap();
        history.append(&sid, b"late producer").unwrap();
        history.flush().unwrap();
        assert!(history_files(&paths, Some(&sid)).is_empty());
    }
    #[test]
    fn age_rotation_and_existing_segments() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let sid = id();
        let legacy = paths.history_dir().join(format!("{sid}-{:020}.pty", now()));
        fs::write(&legacy, b"legacy").unwrap();
        let mut history = History::new(paths.clone()).unwrap();
        history.append(&sid, b"new").unwrap();
        history.writers.get_mut(&sid).unwrap().opened -= std::time::Duration::from_secs(301);
        history.append(&sid, b"rotated").unwrap();
        history.flush().unwrap();
        assert_eq!(history_files(&paths, Some(&sid)).len(), 3);
        history.segments.get_mut(&legacy).unwrap().created = 0;
        history.prune(&Settings::default()).unwrap();
        assert!(!legacy.exists());
    }
    #[test]
    fn sqlite_roundtrip_and_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::at(tmp.path().into());
        p.init().unwrap();
        let (db, mut s) = Store::open(&p).unwrap();
        s.projects.push(Project {
            id: id(),
            name: "test".into(),
            path: "/tmp".into(),
            layout: serde_json::Value::Null,
        });
        db.save(&s).unwrap();
        drop(db);
        let (_, t) = Store::open(&p).unwrap();
        assert_eq!(t.projects.len(), 1);
    }
    #[test]
    fn history_replay_has_no_escape_side_effects() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::at(tmp.path().into());
        p.init().unwrap();
        let sid = id();
        let mut history = History::new(p.clone()).unwrap();
        history
            .append(&sid, b"safe\x1b]52;c;aGVsbG8=\x07\r\ntext")
            .unwrap();
        history.flush().unwrap();
        let s = Session {
            review: false,
            id: sid,
            project_id: id(),
            label: String::new(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Ended,
            created: now(),
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: id(),
            pid: None,
            truncated: false,
            cwd_confirmed: false,
        };
        let t = text(&p, &s).unwrap();
        assert!(t.contains("safe"));
        assert!(!t.contains('\x1b'));
    }
}
