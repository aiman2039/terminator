//! Commit log and blame data structures, fetched via `git log` and `git blame`.
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
#[allow(dead_code)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
    Head,
}

/// Blame annotation for a single line.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameEntry {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub line: usize,
    pub code: String,
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
    let mut commits = Vec::new();
    for entry in text.split(">>>>>END<<<<<\n") {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
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
                .map(|s| s.to_string())
                .collect()
        };
        let refs: Vec<String> = if refs_str.is_empty() {
            vec![]
        } else {
            refs_str.split(", ").map(|s| s.to_string()).collect()
        };
        let timestamp: i64 = date.parse().unwrap_or(0);
        let formatted_date = if timestamp > 0 {
            let secs = timestamp;
            use std::time::SystemTime;
            // approximate: days ago
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let diff = now - secs;
            if diff < 86400 {
                format!("{}h ago", diff / 3600)
            } else if diff < 7 * 86400 {
                format!("{}d ago", diff / 86400)
            } else {
                // Format as YYYY-MM-DD
                let days = secs / 86400;
                let year = 1970 + (days - days / 146097 * 146097 + 719468) / 146097;
                let month = 1;
                let day = 1;
                format!("{year}-{month:02}-{day:02}")
            }
        } else {
            String::new()
        };
        commits.push(CommitEntry {
            hash,
            short_hash,
            author,
            email,
            date: formatted_date,
            timestamp,
            message,
            subject,
            refs,
            parents,
            graph_col: 0,
            graph_color: 0,
        });
    }
    commits
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
            .map(|s| s.to_string())
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

/// Fetch blame annotations for a file.
pub fn fetch_blame(cwd: &Path, path: &Path) -> Result<BlameData, String> {
    let root = git_root(cwd)?;
    let options = CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let bytes = terminator_core::git::run_blocking(
        &root,
        &[
            "blame",
            "--line-porcelain",
            "--",
            path.to_string_lossy().as_ref(),
        ],
        options,
    )
    .map_err(|e| format!("git blame: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut entries = Vec::new();
    let mut current_hash = String::new();
    let mut current_author = String::new();
    let mut current_date = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("author ") {
            current_author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            let ts: i64 = rest.parse().unwrap_or(0);
            current_date = if ts > 0 {
                let secs = ts;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let diff = now - secs;
                if diff < 86400 {
                    format!("{}h ago", diff / 3600)
                } else {
                    format!("{}d ago", diff / 86400)
                }
            } else {
                String::new()
            };
        } else if line.starts_with('\t') {
            let code = line.to_string();
            entries.push(BlameEntry {
                hash: current_hash.clone(),
                author: current_author.clone(),
                date: current_date.clone(),
                line: entries.len(),
                code,
            });
        } else if !line.starts_with(' ')
            && !line.starts_with("author")
            && !line.starts_with("committer")
        {
            // First tab-separated field is the hash+line
            if let Some(hash) = line.split_whitespace().next() {
                current_hash = hash.to_string();
            }
        }
    }
    Ok(BlameData {
        entries,
        error: None,
    })
}
