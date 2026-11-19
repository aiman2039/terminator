//! Agent sessions (#314): the past conversations that Claude Code and Codex
//! keep for one project folder, so the user can resume one by an explicit
//! action. Read-only: only the head of each conversation file (and, for a
//! Claude Code title, a short tail), bounded in files, bytes and time.
//! Credentials are never read and nothing is written. The result is never
//! a lifecycle or presence source.
mod claude;
mod codex;
#[cfg(test)]
mod tests;

pub use claude::claude_dir_name;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Conversations listed at most, newest first.
pub const MAX_LISTED: usize = 100;
/// Conversation files opened at most per listing.
pub const MAX_FILES: usize = 400;
/// Bytes read at most per listing.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// A listing stops after this long and keeps what it found.
pub const TIME_BUDGET: Duration = Duration::from_secs(5);
/// Characters of a title kept.
pub const TITLE_CHARS: usize = 120;
/// Bytes of a Claude Code file read for its folder and first prompt.
const CLAUDE_HEAD: u64 = 128 * 1024;
/// Bytes of a Codex file read for its session meta and first prompt.
const CODEX_HEAD: u64 = 512 * 1024;
/// Bytes at the end of a Claude Code file read for its newest title.
const TITLE_TAIL: u64 = 64 * 1024;
/// Bytes at the end of Codex's `session_index.jsonl` read for titles.
const INDEX_TAIL: u64 = 1024 * 1024;
/// Entries of one Claude Code project folder looked at.
const MAX_DIR_ENTRIES: usize = 5000;

/// One past conversation of an agent CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// Agent kind: `claude` or `codex`.
    pub agent: &'static str,
    /// The CLI's own conversation id; always passes [`valid_id`].
    pub id: String,
    /// The CLI's title, else the first typed prompt, on one line.
    pub title: String,
    /// Unix seconds of the file's last change.
    pub updated: u64,
}

/// Conversations of a folder, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub conversations: Vec<Conversation>,
    /// A file, byte or time limit ran out before every file was read.
    pub stopped: bool,
}

/// Where the agent CLIs keep their conversations.
#[derive(Clone, Debug, Default)]
pub struct Roots {
    /// `~/.claude/projects`.
    pub claude_projects: Option<PathBuf>,
    /// `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
    pub codex_sessions: Option<PathBuf>,
}

impl Roots {
    #[must_use]
    pub fn discover() -> Self {
        let home = directories::BaseDirs::new().map(|base| base.home_dir().to_path_buf());
        Self {
            claude_projects: home.as_ref().map(|h| h.join(".claude").join("projects")),
            codex_sessions: super::quota::codex_sessions_dir(home.as_deref()),
        }
    }
}

/// Bounds of one listing.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub files: usize,
    pub bytes: u64,
    pub time: Duration,
    pub listed: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            files: MAX_FILES,
            bytes: MAX_BYTES,
            time: TIME_BUDGET,
            listed: MAX_LISTED,
        }
    }
}

/// A conversation id that may go into a resume command: ASCII letters,
/// digits, `-` and `_`, starting with a letter or digit, at most 128.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The conversations kept for `folder`, newest first. Blocking file reads:
/// call it off the GUI thread. `stop` ends the listing early.
pub fn list(roots: &Roots, folder: &Path, limits: Limits, stop: &dyn Fn() -> bool) -> Listing {
    let folders = folder_forms(folder);
    let mut budget = Budget {
        files: limits.files,
        bytes: limits.bytes,
        deadline: Instant::now().checked_add(limits.time),
        stop,
        spent: false,
    };
    let mut found = Vec::new();
    if let Some(projects) = &roots.claude_projects {
        found.extend(claude::conversations(
            projects,
            &folders,
            &mut budget,
            limits.listed,
        ));
    }
    if let Some(sessions) = &roots.codex_sessions {
        found.extend(codex::conversations(
            sessions,
            &folders,
            &mut budget,
            limits.listed,
        ));
    }
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated));
    found.truncate(limits.listed);
    Listing {
        conversations: found,
        stopped: budget.spent,
    }
}

/// The folder as given and, when it differs, its canonical path: the CLIs
/// record the folder they started in.
fn folder_forms(folder: &Path) -> Vec<String> {
    let mut forms = vec![folder.to_string_lossy().into_owned()];
    if let Ok(real) = folder.canonicalize() {
        let real = real.to_string_lossy().into_owned();
        if !forms.contains(&real) {
            forms.push(real);
        }
    }
    forms
}

struct Budget<'a> {
    files: usize,
    bytes: u64,
    deadline: Option<Instant>,
    stop: &'a dyn Fn() -> bool,
    spent: bool,
}

impl Budget<'_> {
    /// Takes one file from the budget; false (and `spent`) when a limit
    /// ran out or the listing was stopped.
    fn open(&mut self) -> bool {
        let late = self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if self.files == 0 || self.bytes == 0 || late || (self.stop)() {
            self.spent = true;
            return false;
        }
        self.files = self.files.saturating_sub(1);
        true
    }

    fn charge(&mut self, read: usize) {
        self.bytes = self
            .bytes
            .saturating_sub(u64::try_from(read).unwrap_or(u64::MAX));
    }
}

/// Calls `each` with the lines in the first `limit` bytes of `file` until
/// it returns false. Returns the bytes read.
fn head_lines(
    file: &File,
    limit: u64,
    budget: &mut Budget<'_>,
    mut each: impl FnMut(&str) -> bool,
) -> u64 {
    let mut reader = BufReader::new(file.take(limit.min(budget.bytes)));
    let mut line = Vec::new();
    let mut total: u64 = 0;
    loop {
        line.clear();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            break;
        };
        budget.charge(read);
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if read == 0 || !each(&String::from_utf8_lossy(&line)) {
            break;
        }
    }
    total
}

/// The text from byte `from` (or the last `limit` bytes, if later) to the
/// end of `file`.
fn tail_text(file: &File, from: u64, limit: u64, budget: &mut Budget<'_>) -> String {
    let Ok(length) = file.metadata().map(|meta| meta.len()) else {
        return String::new();
    };
    let limit = limit.min(budget.bytes);
    let start = length.saturating_sub(limit).max(from);
    if start >= length {
        return String::new();
    }
    let mut reader = file;
    if reader.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = reader.take(limit).read_to_end(&mut bytes);
    budget.charge(bytes.len());
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Unix seconds of a file time; 0 when unknown.
fn unix(time: std::io::Result<SystemTime>) -> u64 {
    time.ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |age| age.as_secs())
}

fn modified(file: &File) -> u64 {
    unix(file.metadata().and_then(|meta| meta.modified()))
}

/// One line of text, at most [`TITLE_CHARS`] characters.
fn clean(text: &str) -> Option<String> {
    let words = crate::one_line(text);
    if words.is_empty() {
        return None;
    }
    if words.chars().count() <= TITLE_CHARS {
        return Some(words);
    }
    let mut cut: String = words.chars().take(TITLE_CHARS.saturating_sub(1)).collect();
    cut.push('…');
    Some(cut)
}

/// Typed prompt text; command echoes and injected context start with a
/// `<tag>` and are skipped.
fn prompt_text(text: &str) -> Option<String> {
    let text = text.trim_start();
    if text.starts_with('<') {
        return None;
    }
    clean(text)
}
