//! Commit log and blame data structures, fetched via `git log` and `git blame`.
mod files;
mod filter;
#[cfg(test)]
mod filter_tests;
mod graph;
#[cfg(test)]
mod graph_tests;
mod labels;
mod log_view;
mod query;
#[cfg(test)]
mod tests;
mod web;

pub use files::*;
pub use filter::*;
pub use graph::*;
pub use labels::*;
pub use log_view::*;
pub use query::*;
pub use web::*;

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
    /// HEAD, branches, and tags that point at this commit.
    pub refs: Vec<RefLabel>,
    pub parents: Vec<String>,
    /// This commit's row in the lane graph.
    pub graph: GraphRow,
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
    /// The widest graph row, in lanes.
    pub lanes: usize,
    /// The filters and Go to Hash target this log answers (boxed: the
    /// Git view enum stays small).
    pub query: Box<LogQuery>,
    /// `git config user.name`: the User filter's "me".
    pub me: Option<String>,
    /// Go to Hash: the full hash of the commit it found.
    pub found: Option<String>,
    /// Go to Hash: why it found no commit.
    pub missing: Option<String>,
}

/// Fetched blame data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlameData {
    pub entries: Vec<BlameEntry>,
    pub error: Option<String>,
    /// Commit page prefix on the `origin` host, when it is a known host.
    pub commit_web: Option<String>,
}

/// Fetch the whole commit log (no filters) from a git repository.
pub fn fetch_log(cwd: &Path) -> Result<CommitLog, String> {
    let mut log = query_log(&BlockingGit, cwd, &LogQuery::default());
    match log.error.take() {
        Some(error) => Err(error),
        None => Ok(log),
    }
}

fn git_root(cwd: &Path) -> Result<std::path::PathBuf, String> {
    query::root_with(&BlockingGit, cwd)
}

/// Commits in log order, each with its lane graph row.
#[cfg(test)]
fn parse_log(text: &str) -> Vec<CommitEntry> {
    let mut commits = parse_commits(text);
    query::lay_out(&mut commits, false);
    commits
}

/// Commits in log order, without graph rows.
fn parse_commits(text: &str) -> Vec<CommitEntry> {
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
        refs: parse_ref_labels(&refs_str),
        parents,
        graph: GraphRow::default(),
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
        crate::native_editor::ymd(secs)
    }
}

fn fetch_refs(git: &impl LogGit, root: &Path) -> Vec<RefEntry> {
    let mut refs = Vec::new();
    let options = CommandOptions {
        stdout_limit: 1024 * 1024,
        ..Default::default()
    };
    let bytes = match git.git(
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
