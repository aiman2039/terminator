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
        let formatted_date = if timestamp > 0 {
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
                    .as_secs()
                    .cast_signed();
                let diff = now.saturating_sub(secs);
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

/// One commit in a file's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Commits touching `path`, newest first, following renames.
pub fn file_history(cwd: &Path, path: &Path) -> Result<Vec<HistoryEntry>, String> {
    let root = git_root(cwd)?;
    let relative = terminator_git::relative_path(&root, path).map_err(|error| error.to_string())?;
    let relative = relative.to_string_lossy().into_owned();
    let bytes = terminator_core::git::run_blocking(
        &root,
        &[
            "log",
            "--follow",
            "--date=short",
            "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%s",
            "-300",
            "--",
            &relative,
        ],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    )
    .map_err(|error| format!("git log: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\u{1f}');
            Some(HistoryEntry {
                hash: fields.next()?.to_owned(),
                short_hash: fields.next()?.to_owned(),
                author: fields.next()?.to_owned(),
                date: fields.next()?.to_owned(),
                subject: fields.next().unwrap_or_default().to_owned(),
            })
        })
        .collect())
}

/// Files a commit changed: `(status letter, absolute path)`.
pub fn commit_files(cwd: &Path, hash: &str) -> Result<Vec<(char, std::path::PathBuf)>, String> {
    let root = git_root(cwd)?;
    let bytes = terminator_core::git::run_blocking(
        &root,
        &[
            "show",
            "--name-status",
            "--format=",
            "--first-parent",
            "-m",
            hash,
        ],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    )
    .map_err(|error| format!("git show: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let status = fields.next()?.chars().next()?;
            // Renames and copies list old then new: show the new path.
            let path = fields.next_back()?;
            Some((status, root.join(path)))
        })
        .collect())
}

/// `path` at `revision`, or `None` when it did not exist there.
fn revision_text(root: &Path, revision: &str, relative: &str) -> Option<String> {
    let bytes = terminator_core::git::run_blocking(
        root,
        &["show", &format!("{revision}:{relative}")],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    )
    .ok()?;
    terminator_git::check_text(&bytes).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Read-only diff of `path` between `left` and `right` (empty `right` is
/// the working tree). A side where the file did not exist is empty.
pub fn revision_document(
    cwd: &Path,
    path: &Path,
    left: &str,
    right: &str,
) -> Result<crate::diff::DiffDocument, String> {
    let root = git_root(cwd)?;
    let relative = terminator_git::relative_path(&root, path).map_err(|error| error.to_string())?;
    let name = relative.to_string_lossy().into_owned();
    let old = revision_text(&root, left, &name);
    let new = if right.is_empty() {
        std::fs::read_to_string(root.join(&relative)).ok()
    } else {
        revision_text(&root, right, &name)
    };
    if old.is_none() && new.is_none() {
        return Err(format!("{name} is not a text file in either revision"));
    }
    let mut doc = crate::diff::build_with_context(
        &relative,
        old.as_deref().unwrap_or_default(),
        new.as_deref().unwrap_or_default(),
        false,
        3,
    );
    let short = |revision: &str| {
        if revision.len() > 12 && revision.chars().all(|c| c.is_ascii_hexdigit() || c == '^') {
            revision.chars().take(8).collect()
        } else {
            revision.to_owned()
        }
    };
    doc.left_label = short(left);
    doc.right_label = if right.is_empty() {
        "Working tree".into()
    } else {
        short(right)
    };
    doc.read_only = true;
    Ok(doc)
}

impl crate::GitView {
    /// Run the blocking git query behind a commit-log or blame tab.
    pub(crate) fn load(tab: &crate::Tab) -> Self {
        match tab {
            crate::Tab::CommitLog { cwd } => {
                Self::Log(fetch_log(cwd).unwrap_or_else(|error| CommitLog {
                    error: Some(error),
                    ..Default::default()
                }))
            }
            crate::Tab::Blame { cwd, path } => Self::Blame(fetch_blame(cwd, path)),
            crate::Tab::FileHistory { cwd, path } => Self::History(file_history(cwd, path)),
            crate::Tab::RevisionDiff {
                cwd,
                path,
                left,
                right,
            } => Self::Revision(revision_document(cwd, path, left, right).map(std::sync::Arc::new)),
            _ => Self::failed(tab, "Not a Git view".into()),
        }
    }

    pub(crate) fn failed(tab: &crate::Tab, error: String) -> Self {
        match tab {
            crate::Tab::Blame { .. } => Self::Blame(Err(error)),
            crate::Tab::FileHistory { .. } => Self::History(Err(error)),
            crate::Tab::RevisionDiff { .. } => Self::Revision(Err(error)),
            _ => Self::Log(CommitLog {
                error: Some(error),
                ..Default::default()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_cmd(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_cmd(dir.path(), &["init", "-q", "-b", "main"]);
        git_cmd(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"],
        );
        git_cmd(dir.path(), &["config", "user.name", "Fixture"]);
        dir
    }

    #[test]
    fn fetch_log_reads_commits_refs_and_the_active_branch() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "first commit"]);
        git_cmd(root, &["branch", "feature"]);
        git_cmd(root, &["tag", "v1"]);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        git_cmd(root, &["commit", "-qam", "second commit"]);

        let log = fetch_log(root).expect("log should load");
        assert!(log.error.is_none());
        assert_eq!(log.active_branch, "main");
        assert!(log.commits.iter().any(|c| c.subject == "second commit"));
        assert!(log.commits.iter().any(|c| c.subject == "first commit"));
        assert!(log.commits.iter().all(|c| !c.short_hash.is_empty()));
        assert!(log.commits.iter().all(|c| c.author == "Fixture"));
        assert!(
            log.refs.iter().any(|r| r.name == "feature"),
            "branch ref missing: {:?}",
            log.refs
        );
        assert!(
            log.refs
                .iter()
                .any(|r| r.name == "v1" && r.kind == RefKind::Tag),
            "tag ref missing: {:?}",
            log.refs
        );
    }

    #[test]
    fn fetch_log_reports_a_directory_outside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let log = fetch_log(dir.path());
        assert!(log.is_err(), "non-repository should error: {log:?}");
    }

    #[test]
    fn parse_log_keeps_multi_line_messages_parents_and_refs() {
        let hash = "1111111111111111111111111111111111111111";
        let parent = "2222222222222222222222222222222222222222";
        let text = format!(
            "{hash}\n1111111\nJane\njane@example.invalid\n1700000000\n{parent}\nHEAD -> main, tag: v1\nsubject line\n\nbody line one\nbody line two\n>>>>>END<<<<<\n"
        );
        let commits = parse_log(&text);
        assert_eq!(commits.len(), 1);
        let commit = &commits[0];
        assert_eq!(commit.hash, hash);
        assert_eq!(commit.short_hash, "1111111");
        assert_eq!(commit.author, "Jane");
        assert_eq!(commit.subject, "subject line");
        assert_eq!(
            commit.message,
            "subject line\n\nbody line one\nbody line two"
        );
        assert_eq!(commit.parents, vec![parent.to_string()]);
        assert_eq!(commit.refs, vec!["HEAD -> main", "tag: v1"]);
        assert_eq!(commit.timestamp, 1_700_000_000);
    }

    #[test]
    fn parse_log_ignores_empty_entries() {
        assert!(parse_log("").is_empty());
        assert!(parse_log(">>>>>END<<<<<\n").is_empty());
    }

    #[test]
    fn fetch_blame_annotates_every_line() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("blame.txt"), "alpha\nbeta\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "base"]);

        let blame = fetch_blame(root, Path::new("blame.txt")).expect("blame should load");
        assert!(blame.error.is_none());
        assert_eq!(blame.entries.len(), 2);
        assert_eq!(blame.entries[0].line, 0);
        assert_eq!(blame.entries[1].line, 1);
        assert!(blame.entries.iter().all(|e| e.author == "Fixture"));
        assert!(blame.entries[0].code.contains("alpha"));
        assert!(blame.entries[1].code.contains("beta"));
    }

    #[test]
    fn history_files_and_revision_diffs_follow_commits() {
        let dir = repo();
        let root = dir.path();
        let file = root.join("a.txt");
        std::fs::write(&file, "one\n").unwrap();
        git_cmd(root, &["add", "a.txt"]);
        git_cmd(root, &["commit", "-qm", "add a"]);
        std::fs::write(&file, "one\ntwo\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "grow a, add b"]);
        std::fs::write(&file, "one\ntwo\nlocal\n").unwrap();

        let history = file_history(root, &file).unwrap();
        assert_eq!(
            history
                .iter()
                .map(|entry| entry.subject.as_str())
                .collect::<Vec<_>>(),
            ["grow a, add b", "add a"]
        );
        let newest = &history[0].hash;
        let mut changed = commit_files(root, newest).unwrap();
        changed.sort();
        let root_real = root.canonicalize().unwrap();
        assert_eq!(
            changed,
            [
                ('A', root_real.join("b.txt")),
                ('M', root_real.join("a.txt"))
            ]
        );

        // The commit's own change: one added line.
        let commit = revision_document(root, &file, &format!("{newest}^"), newest).unwrap();
        assert_eq!(commit.hunks.len(), 1);
        assert_eq!(commit.right_text, "one\ntwo\n");
        assert!(commit.read_only);
        // Against the working tree.
        let local = revision_document(root, &file, newest, "").unwrap();
        assert_eq!(local.right_label, "Working tree");
        assert_eq!(local.right_text, "one\ntwo\nlocal\n");
        // A file added by the commit has an empty left side.
        let added =
            revision_document(root, &root.join("b.txt"), &format!("{newest}^"), newest).unwrap();
        assert_eq!(added.left_text, "");
        assert_eq!(added.right_text, "b\n");
    }
}
