//! Commit log and blame data structures, fetched via `git log` and `git blame`.
mod files;
#[cfg(test)]
mod tests;

pub use files::*;

use std::path::Path;
pub use terminator_core::CommandOptions;

/// A single commit in the log.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub timestamp: i64,
    pub message: String,
    pub subject: String,
    pub refs: Vec<String>,
    pub parents: Vec<String>,
    /// Graph edges: for each parent index (into the visible commit list),
    /// the lane/column this commit connects to. Empty for initial commits.
    pub graph_col: usize,
    pub graph_color: u8,
}

/// A branch or tag reference.
#[derive(Clone, Debug, PartialEq)]
pub struct RefEntry {
    pub name: String,
    pub full_name: String,
    pub kind: RefKind,
    pub target_hash: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// Blame annotation for a single line.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameEntry {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub line: usize,
    pub code: String,
    /// Author time (Unix seconds; 0 when unknown).
    pub time: i64,
    /// The commit's subject line.
    pub summary: String,
}

impl BlameEntry {
    /// Lines not in any commit yet (`git blame --contents`).
    #[must_use]
    pub fn uncommitted(&self) -> bool {
        !self.hash.is_empty() && self.hash.bytes().all(|byte| byte == b'0')
    }
}

/// Fetched commit log for a repository.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommitLog {
    pub commits: Vec<CommitEntry>,
    pub refs: Vec<RefEntry>,
    pub active_branch: String,
    pub error: Option<String>,
}

/// Fetched blame data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlameData {
    pub entries: Vec<BlameEntry>,
    pub error: Option<String>,
}

/// Fetch the commit log from a git repository.
pub fn fetch_log(cwd: &Path) -> Result<CommitLog, String> {
    let root = match git_root(cwd) {
        Ok(r) => r,
        Err(e) => return Err(format!("Git root: {e}")),
    };
    let branch = match current_branch(&root) {
        Ok(b) => b,
        Err(e) => return Err(format!("Branch: {e}")),
    };
    let log_output = match terminator_core::git::run_blocking(
        &root,
        &[
            "log",
            "--all",
            "--date=unix",
            "--format=%H%n%h%n%an%n%ae%n%ad%n%P%n%D%n%B%n>>>>>END<<<<<",
            "-1000",
        ],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    ) {
        Ok(v) => v,
        Err(e) => return Err(format!("git log: {e}")),
    };
    let log_text = String::from_utf8_lossy(&log_output);
    let commits = parse_log(&log_text);
    let refs = fetch_refs(&root);
    Ok(CommitLog {
        commits,
        refs,
        active_branch: branch,
        error: None,
    })
}

fn git_root(cwd: &Path) -> Result<std::path::PathBuf, String> {
    let options = CommandOptions {
        stdout_limit: 1024 * 1024,
        ..Default::default()
    };
    let bytes = terminator_core::git::run_blocking(cwd, &["rev-parse", "--show-toplevel"], options)
        .map_err(|e| format!("{e}"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| format!("{e}"))?
        .trim();
    Ok(std::path::PathBuf::from(text))
}

fn current_branch(root: &Path) -> Result<String, String> {
    let options = CommandOptions {
        stdout_limit: 1024 * 1024,
        ..Default::default()
    };
    let bytes =
        terminator_core::git::run_blocking(root, &["rev-parse", "--abbrev-ref", "HEAD"], options)
            .map_err(|e| format!("{e}"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| format!("{e}"))?
        .trim();
    Ok(text.to_string())
}

fn parse_log(text: &str) -> Vec<CommitEntry> {
    text.split(">>>>>END<<<<<\n")
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_commit)
        .collect()
}

/// One `git log` record: hash, short hash, author, email, date, parents,
/// refs, then the message.
fn parse_commit(entry: &str) -> CommitEntry {
    let mut lines = entry.lines();
    let hash = lines.next().unwrap_or("").to_string();
    let short_hash = lines.next().unwrap_or("").to_string();
    let author = lines.next().unwrap_or("").to_string();
    let email = lines.next().unwrap_or("").to_string();
    let date = lines.next().unwrap_or("").to_string();
    let parents_str = lines.next().unwrap_or("").to_string();
    let refs_str = lines.next().unwrap_or("").to_string();
    let message = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let subject = message.lines().next().unwrap_or("").to_string();
    let parents: Vec<String> = if parents_str.is_empty() {
        vec![]
    } else {
        parents_str
            .split_whitespace()
            .map(std::string::ToString::to_string)
            .collect()
    };
    let refs: Vec<String> = if refs_str.is_empty() {
        vec![]
    } else {
        refs_str
            .split(", ")
            .map(std::string::ToString::to_string)
            .collect()
    };
    let timestamp: i64 = date.parse().unwrap_or(0);
    CommitEntry {
        hash,
        short_hash,
        author,
        email,
        date: relative_date(timestamp),
        timestamp,
        message,
        subject,
        refs,
        parents,
        graph_col: 0,
        graph_color: 0,
    }
}

/// Hours or days ago for recent commits, else a date; blank when unknown.
fn relative_date(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    let secs = timestamp;
    use std::time::SystemTime;
    // approximate: days ago
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed();
    let diff = now.saturating_sub(secs);
    if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else if diff < 7 * 86400 {
        format!("{}d ago", diff / 86400)
    } else {
        // Format as YYYY-MM-DD
        let days = secs / 86400;
        let span = 146_097_i64;
        let year = days
            .checked_div(span)
            .and_then(|quotient| quotient.checked_mul(span))
            .and_then(|product| days.checked_sub(product))
            .and_then(|remainder| remainder.checked_add(719_468))
            .and_then(|shifted| shifted.checked_div(span))
            .and_then(|era| era.checked_add(1970))
            .unwrap_or(1970);
        let month = 1;
        let day = 1;
        format!("{year}-{month:02}-{day:02}")
    }
}

fn fetch_refs(root: &Path) -> Vec<RefEntry> {
    let mut refs = Vec::new();
    let options = CommandOptions {
        stdout_limit: 1024 * 1024,
        ..Default::default()
    };
    let bytes = match terminator_core::git::run_blocking(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)%09%(objectname)%09%(upstream)%09%(upstream:track)",
            "refs/heads/",
            "refs/remotes/",
            "refs/tags/",
        ],
        options,
    ) {
        Ok(v) => v,
        Err(_) => return refs,
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let mut parts = line.split('\t');
        let full_name = parts.next().unwrap_or("").to_string();
        let hash = parts.next().unwrap_or("").to_string();
        let kind = if full_name.starts_with("refs/heads/") {
            RefKind::LocalBranch
        } else if full_name.starts_with("refs/remotes/") {
            RefKind::RemoteBranch
        } else {
            RefKind::Tag
        };
        let name = full_name
            .strip_prefix("refs/heads/")
            .or_else(|| full_name.strip_prefix("refs/remotes/"))
            .or_else(|| full_name.strip_prefix("refs/tags/"))
            .unwrap_or(&full_name)
            .to_string();
        let upstream = parts
            .next()
            .map(std::string::ToString::to_string)
            .filter(|s| !s.is_empty());
        refs.push(RefEntry {
            name,
            full_name,
            kind,
            target_hash: hash,
            upstream,
            ahead: 0,
            behind: 0,
        });
    }
    refs
}
