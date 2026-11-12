use anyhow::{Result, anyhow, bail};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};
use terminator_core::*;

mod decode;
mod store;
pub use store::Store;
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
/// A segment rotates at this size or age.
const SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
const SEGMENT_AGE: std::time::Duration = std::time::Duration::from_mins(5);
struct Segment {
    session: String,
    size: u64,
    created: u64,
}
struct Writer {
    file: std::io::BufWriter<fs::File>,
    path: PathBuf,
    opened: std::time::Instant,
    written: std::time::Instant,
}
/// All retained-file mutations are owned by the history worker.
pub struct History {
    paths: Paths,
    writers: std::collections::HashMap<String, Writer>,
    /// Last segment of each idle-closed writer, reopened while young and
    /// small so a pause between commands does not start a new file.
    idle: std::collections::HashMap<String, (PathBuf, std::time::Instant)>,
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
            idle: std::collections::HashMap::default(),
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
        const LIMIT: u64 = SEGMENT_BYTES;
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
        let writer = self
            .writers
            .get_mut(session)
            .ok_or_else(|| anyhow!("Missing history writer"))?;
        writer.file.write_all(chunk)?;
        writer.written = std::time::Instant::now();
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
            size >= limit || writer.opened.elapsed() >= SEGMENT_AGE
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
        if let Some((path, opened)) = self.idle.remove(session)
            && opened.elapsed() < SEGMENT_AGE
            && self
                .segments
                .get(&path)
                .is_some_and(|s| s.size < SEGMENT_BYTES)
            && let Ok(file) = fs::OpenOptions::new().append(true).open(&path)
        {
            let file = std::io::BufWriter::with_capacity(65536, file);
            let written = std::time::Instant::now();
            let writer = Writer {
                file,
                path,
                opened,
                written,
            };
            self.writers.insert(session.into(), writer);
            return Ok(());
        }
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
                written: std::time::Instant::now(),
            },
        );
        Ok(())
    }
    /// Flushes all writers and closes those idle for 30 s, so ended
    /// sessions do not keep a file and buffer open (#238).
    pub fn flush(&mut self) -> Result<()> {
        self.flush_closing_idle(std::time::Duration::from_secs(30))
    }
    fn flush_closing_idle(&mut self, idle: std::time::Duration) -> Result<()> {
        for writer in self.writers.values_mut() {
            writer.file.flush()?;
        }
        let closed = self.writers.extract_if(|_, w| w.written.elapsed() >= idle);
        let closed: Vec<_> = closed.map(|(s, w)| (s, (w.path, w.opened))).collect();
        self.idle.extend(closed);
        self.idle
            .retain(|_, (_, opened)| opened.elapsed() < SEGMENT_AGE);
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
        // A file removed by hand is already gone: forget it and go on, or
        // every later prune fails here and the budget stops working.
        match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let segment = self
            .segments
            .remove(path)
            .ok_or_else(|| anyhow!("Missing history segment"))?;
        self.total = self.total.saturating_sub(segment.size);
        if let Some(size) = self.per_session.get_mut(&session) {
            *size = size.saturating_sub(segment.size);
            if *size == 0 && !self.segments.values().any(|s| s.session == session) {
                self.per_session.remove(&session);
            }
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
            let session_usage = self.per_session.get(&session).copied().unwrap_or(0);
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
    Ok(screen_text(parser.screen().clone(), 10_000))
}
pub fn screen_text(mut screen: vt100::Screen, limit: usize) -> String {
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
