//! Per-file history: blame, file log, commit files and revision snapshots.
use super::*;

/// Fetch blame annotations for a file.
pub fn fetch_blame(cwd: &Path, path: &Path) -> Result<BlameData, String> {
    blame_with_contents(cwd, path, None)
}

/// Blame `path` as `contents` (an editor's unsaved text) when given, so
/// every buffer line gets an entry and edited lines read as uncommitted.
pub fn blame_with_contents(
    cwd: &Path,
    path: &Path,
    contents: Option<&str>,
) -> Result<BlameData, String> {
    let root = git_root(cwd)?;
    let options = CommandOptions {
        stdout_limit: 16 * 1024 * 1024,
        input: contents.map(|text| text.as_bytes().to_vec()),
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    let path = path.to_string_lossy();
    let mut args = vec!["blame", "--line-porcelain"];
    if contents.is_some() {
        args.extend(["--contents", "-"]);
    }
    args.extend(["--", path.as_ref()]);
    let bytes = terminator_core::git::run_blocking(&root, &args, options)
        .map_err(|e| format!("git blame: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut entries = Vec::new();
    let mut current_hash = String::new();
    let mut current_author = String::new();
    let mut current_date = String::new();
    let mut current_time = 0i64;
    let mut current_summary = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("summary ") {
            rest.clone_into(&mut current_summary);
        } else if let Some(rest) = line.strip_prefix("author ") {
            current_author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            let ts: i64 = rest.parse().unwrap_or(0);
            current_time = ts;
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
                time: current_time,
                summary: current_summary.clone(),
            });
        } else if let Some(hash) = line.split_whitespace().next()
            && hash.len() >= 40
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            // `<hash> <original line> <final line> [<group size>]`; header
            // lines (`summary`, `filename`, `previous`, …) are not hashes.
            hash.clone_into(&mut current_hash);
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

/// The name `relative` (its current name) had at `revision`, following
/// renames: `C` is the name after commit `C`, `C^` the name before it.
/// Other revisions, and files never renamed, keep `relative`.
fn name_at(root: &Path, relative: &str, revision: &str) -> String {
    if revision.is_empty() || revision_text(root, revision, relative).is_some() {
        return relative.to_owned();
    }
    let (commit, before) = match revision.strip_suffix('^') {
        Some(commit) => (commit, true),
        None => (revision, false),
    };
    let Ok(bytes) = terminator_core::git::run_blocking(
        root,
        &[
            "log",
            "--follow",
            "--name-status",
            "--format=%x1e%H",
            "--",
            relative,
        ],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    ) else {
        return relative.to_owned();
    };
    let text = String::from_utf8_lossy(&bytes);
    for record in text.split('\u{1e}') {
        let mut lines = record.lines().filter(|line| !line.trim().is_empty());
        let Some(hash) = lines.next() else {
            continue;
        };
        if !hash.starts_with(commit) {
            continue;
        }
        // `R100\told\tnew` for a rename, `M\tpath` otherwise.
        let Some(status) = lines.next() else {
            break;
        };
        let fields: Vec<&str> = status.split('\t').collect();
        let name = match fields.as_slice() {
            [kind, old, new] if kind.starts_with(['R', 'C']) => {
                if before {
                    old
                } else {
                    new
                }
            }
            [_, path] => path,
            _ => break,
        };
        return (*name).to_owned();
    }
    relative.to_owned()
}

pub struct RevisionDocumentInput<'a> {
    pub cwd: &'a Path,
    pub path: &'a Path,
    pub left: &'a str,
    pub right: &'a str,
}

/// Read-only diff of `path` between `left` and `right` (empty `right` is
/// the working tree). A side where the file did not exist is empty; a
/// side from before a rename reads the file under its name back then.
pub fn revision_document(
    input: RevisionDocumentInput<'_>,
) -> Result<crate::diff::DiffDocument, String> {
    let RevisionDocumentInput {
        cwd,
        path,
        left,
        right,
    } = input;
    let root = git_root(cwd)?;
    let relative = terminator_git::relative_path(&root, path).map_err(|error| error.to_string())?;
    let name = relative.to_string_lossy().into_owned();
    let left_name = name_at(&root, &name, left);
    let right_name = name_at(&root, &name, right);
    let old = revision_text(&root, left, &left_name);
    let new = if right.is_empty() {
        std::fs::read_to_string(root.join(&relative)).ok()
    } else {
        revision_text(&root, right, &right_name)
    };
    if old.is_none() && new.is_none() {
        return Err(format!("{name} is not a text file in either revision"));
    }
    let mut doc = crate::diff::build_with_context(crate::diff::BuildWithContextInput {
        path: &relative,
        left: old.as_deref().unwrap_or_default(),
        right: new.as_deref().unwrap_or_default(),
        staged: false,
        context: 3,
    });
    let short = |revision: &str| {
        if revision.len() > 12 && revision.chars().all(|c| c.is_ascii_hexdigit() || c == '^') {
            revision.chars().take(8).collect()
        } else {
            revision.to_owned()
        }
    };
    // A renamed side says which name it shows (`IntelliJ`: old → new).
    let labelled = |label: String, side: &str| {
        if side == name {
            label
        } else {
            format!("{label} · {side}")
        }
    };
    doc.left_label = labelled(short(left), &left_name);
    doc.right_label = labelled(
        if right.is_empty() {
            "Working tree".into()
        } else {
            short(right)
        },
        &right_name,
    );
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
            } => Self::Revision(
                revision_document(RevisionDocumentInput {
                    cwd,
                    path,
                    left,
                    right,
                })
                .map(std::sync::Arc::new),
            ),
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
