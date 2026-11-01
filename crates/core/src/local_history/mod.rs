//! Local History: content snapshots of project files, kept for a number of
//! days. A safety net for edits that Git does not see yet, mainly while
//! agents change files.
//!
//! Layout under the store root (`$data/local-history`, mode 0700):
//! - `blobs/<sha256>`: file contents, addressed by hash (mode 0600).
//! - `files/<sha256 of path>.jsonl`: one [`Entry`] per line, oldest first.
//! - `labels.jsonl`: project-wide markers such as "before agent run".
//! - `lock`: writers hold an exclusive lock; readers skip torn lines.
//!
//! Entry ids are content hashes, so a caller can never name a path outside
//! the store.
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

/// Daemons that serve `LocalHistory*` requests advertise this.
pub const CAPABILITY: &str = "local-history-v1";
/// Folder under the shared data directory.
pub const DIR: &str = "local-history";
/// Larger files are not recorded.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Label for the snapshot the daemon takes when an agent starts a run.
pub const BEFORE_AGENT_RUN: &str = "Before agent run";
/// Label for the content a revert replaces.
pub const BEFORE_REVERT: &str = "Before revert";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Why a snapshot was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Written by an editor save that the daemon saw.
    Save,
    /// A change on disk seen by the file watcher.
    External,
    /// The oldest known content: the Git index version when the watcher first
    /// sees the file change.
    Open,
    /// A project-wide marker, not a file snapshot.
    Label,
}

/// One timeline row. `id` is the content hash; it is empty for a label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub path: PathBuf,
    pub timestamp_ms: u64,
    pub size: u64,
    #[serde(default)]
    pub label: Option<String>,
    pub source: Source,
}

#[derive(Debug, Serialize, Deserialize)]
struct LabelRecord {
    project_root: PathBuf,
    timestamp_ms: u64,
    label: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalHistoryError {
    #[error("Local history I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Local history index: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid local history id")]
    InvalidId,
    #[error("Local history snapshot is missing")]
    Missing,
    #[error("Local history snapshot is not UTF-8 text")]
    NotText,
}
type Result<T> = std::result::Result<T, LocalHistoryError>;

/// What one snapshot records.
pub struct Record<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
    pub source: Source,
    pub label: Option<&'a str>,
    pub now_ms: u64,
}

/// Counts from [`Store::prune`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub entries: usize,
    pub blobs: usize,
}

/// Whether `bytes` is worth keeping: UTF-8 text with no NUL, at most
/// [`MAX_BYTES`].
#[must_use]
pub fn recordable(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_BYTES && !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Lowercase hex SHA-256.
#[must_use]
pub fn hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    use std::fmt::Write as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// The on-disk store. Cheap to construct; it opens files per call.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The store under a data directory.
    #[must_use]
    pub fn in_data(data: &Path) -> Self {
        Self::new(data.join(DIR))
    }

    /// Saves `bytes` as the newest snapshot of `path`. Returns `None` when
    /// the content is not recordable or equals the newest snapshot.
    pub fn record(&self, record: Record<'_>) -> Result<Option<Entry>> {
        if !recordable(record.bytes) {
            return Ok(None);
        }
        let _lock = self.lock()?;
        let id = hash(record.bytes);
        let index = self.index_path(record.path);
        if read_lines::<Entry>(&index)?.last().map(|e| e.id.as_str()) == Some(id.as_str()) {
            return Ok(None);
        }
        let blob = self.root.join("blobs").join(&id);
        if !blob.exists() {
            private_dir(&self.root.join("blobs"))?;
            crate::atomic_write(&blob, record.bytes).map_err(std::io::Error::other)?;
        }
        let entry = Entry {
            id,
            path: record.path.to_path_buf(),
            timestamp_ms: record.now_ms,
            size: u64::try_from(record.bytes.len()).unwrap_or(u64::MAX),
            label: record.label.map(str::to_owned),
            source: record.source,
        };
        append_line(&index, &entry)?;
        Ok(Some(entry))
    }

    /// Whether any snapshot of `path` exists.
    #[must_use]
    pub fn has_history(&self, path: &Path) -> bool {
        self.index_path(path).exists()
    }

    /// Snapshots of `path` plus labels of projects that contain it, newest
    /// first.
    pub fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        let mut entries: Vec<Entry> = read_lines(&self.index_path(path))?;
        entries.extend(
            read_lines::<LabelRecord>(&self.root.join("labels.jsonl"))?
                .into_iter()
                .filter(|label| path.starts_with(&label.project_root))
                .map(|label| Entry {
                    id: String::new(),
                    path: path.to_path_buf(),
                    timestamp_ms: label.timestamp_ms,
                    size: 0,
                    label: Some(label.label),
                    source: Source::Label,
                }),
        );
        // Stable sort keeps a later line after an earlier one at equal time.
        entries.reverse();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        Ok(entries)
    }

    /// The text of snapshot `id`.
    pub fn read(&self, id: &str) -> Result<String> {
        if !valid_id(id) {
            return Err(LocalHistoryError::InvalidId);
        }
        let bytes = match fs::read(self.root.join("blobs").join(id)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(LocalHistoryError::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        String::from_utf8(bytes).map_err(|_| LocalHistoryError::NotText)
    }

    /// Adds a marker shown in the timeline of every file under `project_root`.
    pub fn label(&self, project_root: &Path, label: &str, now_ms: u64) -> Result<()> {
        let _lock = self.lock()?;
        append_line(
            &self.root.join("labels.jsonl"),
            &LabelRecord {
                project_root: project_root.to_path_buf(),
                timestamp_ms: now_ms,
                label: label.to_owned(),
            },
        )
    }

    /// Drops entries and labels older than `max_age_days`, then blobs no
    /// entry uses. `0` days keeps everything (recording is off).
    pub fn prune(&self, now_ms: u64, max_age_days: u32) -> Result<Pruned> {
        let mut pruned = Pruned::default();
        if max_age_days == 0 || !self.root.exists() {
            return Ok(pruned);
        }
        let _lock = self.lock()?;
        let cutoff = now_ms.saturating_sub(DAY_MS.saturating_mul(u64::from(max_age_days)));
        let mut used = HashSet::new();
        for index in list_dir(&self.root.join("files"))? {
            let entries: Vec<Entry> = read_lines(&index)?;
            let total = entries.len();
            let kept: Vec<Entry> = entries
                .into_iter()
                .filter(|e| e.timestamp_ms >= cutoff)
                .collect();
            pruned.entries = pruned
                .entries
                .saturating_add(total.saturating_sub(kept.len()));
            used.extend(kept.iter().map(|e| e.id.clone()));
            if kept.is_empty() {
                fs::remove_file(&index)?;
            } else if kept.len() != total {
                write_lines(&index, &kept)?;
            }
        }
        let labels_path = self.root.join("labels.jsonl");
        let labels: Vec<LabelRecord> = read_lines(&labels_path)?;
        let total = labels.len();
        let labels: Vec<LabelRecord> = labels
            .into_iter()
            .filter(|l| l.timestamp_ms >= cutoff)
            .collect();
        if labels.len() != total {
            write_lines(&labels_path, &labels)?;
        }
        for blob in list_dir(&self.root.join("blobs"))? {
            let name = blob.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !used.contains(name) {
                fs::remove_file(&blob)?;
                pruned.blobs = pruned.blobs.saturating_add(1);
            }
        }
        Ok(pruned)
    }

    fn index_path(&self, path: &Path) -> PathBuf {
        self.root.join("files").join(format!(
            "{}.jsonl",
            hash(path.as_os_str().as_encoded_bytes())
        ))
    }

    fn lock(&self) -> Result<fs::File> {
        private_dir(&self.root)?;
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(self.root.join("lock"))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(file)
    }
}

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn list_dir(path: &Path) -> Result<Vec<PathBuf>> {
    match fs::read_dir(path) {
        Ok(entries) => Ok(entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

/// Parsed lines; a torn or foreign line is skipped, never fatal.
fn read_lines<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    Ok(std::io::BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect())
}

fn append_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)?.write_all(&line)?;
    Ok(())
}

fn write_lines<T: Serialize>(path: &Path, values: &[T]) -> Result<()> {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend(serde_json::to_vec(value)?);
        bytes.push(b'\n');
    }
    crate::atomic_write(path, &bytes).map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests;
