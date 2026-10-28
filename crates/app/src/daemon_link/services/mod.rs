mod context;
mod listing;

pub use context::context_async;
pub(crate) use listing::Listing;
pub use listing::{directories_with_ignored_async, directory_raw_async};
#[cfg(test)]
mod tests;

use anyhow::{Result, ensure};
use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};
#[cfg(test)]
use terminator_core::CommandOptions;
use terminator_git::{Change, Entry};

/// Called by the folder-opening worker, including for saved symlink aliases.
pub fn project_for_directory<'a>(
    projects: &'a [terminator_core::Project],
    directory: &Path,
) -> Option<&'a terminator_core::Project> {
    projects
        .iter()
        .find(|p| p.path == directory || p.path.canonicalize().is_ok_and(|path| path == directory))
}

#[derive(Clone, Debug)]
pub struct ContextData {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub perforce: Option<crate::perforce::Workspace>,
    pub git_dirs: Vec<PathBuf>,
    pub branch: String,
    pub changes: Vec<Change>,
    pub decorations: std::collections::HashMap<PathBuf, char>,
    /// Added/deleted line counts per changed path (index plus worktree).
    pub stats: std::collections::HashMap<PathBuf, (u32, u32)>,
    pub error: Option<String>,
}

/// Merge `--numstat` output for the index and worktree, then add line counts
/// for untracked files. Paths match the porcelain records so the Git rows can
/// look stats up directly.
#[cfg(test)]
pub fn merge_stats(
    root: &Path,
    working: &[u8],
    staged: &[u8],
    changes: &[Change],
) -> std::collections::HashMap<PathBuf, (u32, u32)> {
    merge_stats_cancel(MergeStatsCancelInput {
        root,
        working,
        staged,
        changes,
        cancel: None,
    })
}

struct MergeStatsCancelInput<'a> {
    root: &'a Path,
    working: &'a [u8],
    staged: &'a [u8],
    changes: &'a [Change],
    cancel: Option<&'a terminator_core::async_service::CancellationToken>,
}

fn merge_stats_cancel(
    input: MergeStatsCancelInput<'_>,
) -> std::collections::HashMap<PathBuf, (u32, u32)> {
    let MergeStatsCancelInput {
        root,
        working,
        staged,
        changes,
        cancel,
    } = input;
    let mut stats = terminator_git::parse_numstat(root, staged);
    for (path, (added, deleted)) in terminator_git::parse_numstat(root, working) {
        let entry = stats.entry(path).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(added);
        entry.1 = entry.1.saturating_add(deleted);
    }
    for change in changes.iter().filter(|change| change.status == "??") {
        if cancel.is_some_and(terminator_core::async_service::CancellationToken::is_cancelled) {
            break;
        }
        let Ok(metadata) = fs::symlink_metadata(&change.path) else {
            continue;
        };
        const MAX_UNTRACKED_BYTES: u64 = 2 * 1024 * 1024;
        if !metadata.file_type().is_file() || metadata.len() > MAX_UNTRACKED_BYTES {
            continue;
        }
        let Ok(file) = fs::File::open(&change.path) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_UNTRACKED_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > MAX_UNTRACKED_BYTES
            || bytes.contains(&0)
        {
            continue;
        }
        let mut lines =
            u32::try_from(bytes.iter().filter(|byte| **byte == b'\n').count()).unwrap_or(u32::MAX);
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            lines = lines.saturating_add(1);
        }
        let slot = stats.entry(change.path.clone()).or_insert((0, 0));
        slot.0 = slot.0.saturating_add(lines);
    }
    stats
}

/// Parsed `git --numstat` stats for the worktree and index. Missing paths are
/// empty, which is normal for binary files and ignored directories.
#[cfg(test)]
pub fn context_stats(
    cwd: &Path,
    root: &Path,
    changes: &[Change],
) -> std::collections::HashMap<PathBuf, (u32, u32)> {
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let git =
        |args: &[&str]| terminator_git::run_blocking(cwd, args, options()).unwrap_or_default();
    let working = git(&["diff", "--numstat", "-z"]);
    let staged = git(&["diff", "--cached", "--numstat", "-z"]);
    merge_stats(root, &working, &staged, changes)
}

#[derive(Clone, Debug)]
pub struct DirectoryError {
    pub path: PathBuf,
    pub kind: std::io::ErrorKind,
    pub message: String,
}

#[cfg(test)]
pub fn directory_result(
    path: &Path,
    in_git: bool,
) -> std::result::Result<Vec<Entry>, DirectoryError> {
    terminator_git::entries_known(path, in_git).map_err(|error| DirectoryError {
        path: path.into(),
        kind: match &error {
            terminator_git::Error::Io(inner) => inner.kind(),
            _ => std::io::ErrorKind::Other,
        },
        message: error.to_string(),
    })
}

pub fn entries_raw(
    path: &Path,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        anyhow::ensure!(!cancel.is_cancelled(), "Directory scan cancelled");
        let entry = entry?;
        anyhow::ensure!(
            entries.len() < terminator_git::DISPLAY_LIMIT,
            "Directory exceeds the {}-entry display limit",
            terminator_git::DISPLAY_LIMIT
        );
        entries.push(Entry {
            ignored: entry.file_name() == ".git",
            path: entry.path(),
            directory: entry.file_type()?.is_dir(),
        });
    }
    entries.sort_by(terminator_git::compare_entries);
    Ok(entries)
}

#[cfg(test)]
pub fn context_cached(
    cwd: &Path,
    cache: &mut std::collections::HashMap<PathBuf, (PathBuf, Vec<PathBuf>)>,
    include_git_status: bool,
) -> ContextData {
    let mut result = ContextData {
        cwd: cwd.into(),
        root: None,
        perforce: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let git = |cwd: &Path, args: &[&str]| terminator_git::run_blocking(cwd, args, options());
    let (root, metadata) = if let Some(cached) = cache.get(cwd) {
        cached.clone()
    } else {
        let root = match git(cwd, &["rev-parse", "--show-toplevel"]) {
            Ok(v) => {
                let root = PathBuf::from(String::from_utf8_lossy(&v).trim());
                // Match filesystem paths, including Windows verbatim prefixes.
                root.canonicalize().unwrap_or(root)
            }
            Err(_) => return result,
        };
        let metadata = ["--absolute-git-dir", "--git-common-dir"]
            .into_iter()
            .filter_map(|arg| git(cwd, &["rev-parse", arg]).ok())
            .map(|v| {
                let p = PathBuf::from(String::from_utf8_lossy(&v).trim());
                if p.is_absolute() { p } else { cwd.join(p) }
            })
            .collect::<Vec<_>>();
        cache.insert(cwd.into(), (root.clone(), metadata.clone()));
        (root, metadata)
    };
    result.git_dirs = metadata;
    result.root = Some(root.clone());
    if !include_git_status {
        return result;
    }
    result.branch = git(&root, &["branch", "--show-current"])
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    if result.branch.is_empty() {
        result.branch = "Detached HEAD".into();
    }
    match git(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) {
        Ok(raw) => {
            result.changes = terminator_git::parse_porcelain(&root, &raw);
        }
        Err(e) => result.error = Some(e.to_string()),
    }
    result.decorations = terminator_git::decorations(&root, &result.changes);
    result.stats = context_stats(&result.cwd, &root, &result.changes);
    result
}

#[derive(Clone, Debug)]
pub enum Target {
    File(PathBuf, Option<u32>, Option<u32>),
    Url(String),
}
impl Target {
    pub fn display(&self) -> String {
        self.render(|path| path.display().to_string())
    }

    pub fn compact(&self) -> String {
        self.render(compact_path)
    }

    fn render(&self, path_text: impl Fn(&Path) -> String) -> String {
        match self {
            Self::File(path, Some(line), Some(column)) => {
                format!("{}:{line}:{column}", path_text(path))
            }
            Self::File(path, Some(line), None) => format!("{}:{line}", path_text(path)),
            Self::File(path, None, _) => path_text(path),
            Self::Url(url) => url.clone(),
        }
    }
}

/// `$HOME/foo` → `~/foo` for labels. Copy and open still use the real path.
pub fn compact_path(path: &Path) -> String {
    compact_path_under(path, std::env::var_os("HOME").as_deref().map(Path::new))
}

pub(crate) fn compact_path_under(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) else {
        return path.display().to_string();
    };
    match path.strip_prefix(home) {
        Ok(relative) if relative.as_os_str().is_empty() => "~".into(),
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn resolve_target(text: &str, cwd: &Path) -> Result<Target> {
    if (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
    {
        ensure!(
            text.split_once("://")
                .is_some_and(|(_, host)| !host.is_empty()),
            "URL has no host"
        );
        return Ok(Target::Url(text.into()));
    }
    ensure!(
        !cwd.starts_with("/__terminator_remote__"),
        "Remote paths must be opened through the SSH project explorer"
    );
    let (path, line) = resolve_path(text, cwd)?;
    ensure!(path.is_file(), "Target is not a file");
    let mut pieces = text.rsplit(':');
    let last = pieces.next().and_then(|n| n.parse::<u32>().ok());
    let column = if line.is_some() && pieces.next().is_some_and(|n| n.parse::<u32>().is_ok()) {
        last
    } else {
        None
    };
    Ok(Target::File(path, line, column))
}
fn target_path(text: &str, cwd: &Path) -> PathBuf {
    let text = text.trim_matches(['\'', '"', '`']);
    if let Some(relative) = text.strip_prefix("~/") {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(relative);
    }
    let text = text.strip_prefix("file://").unwrap_or(text);
    if Path::new(text).is_absolute() {
        PathBuf::from(text)
    } else {
        cwd.join(text)
    }
}

pub fn resolve_path(text: &str, cwd: &Path) -> Result<(PathBuf, Option<u32>)> {
    let text = text.trim().trim_matches(['\'', '"', '`']);
    let exact = target_path(text, cwd);
    if exact.exists() {
        return Ok((exact.canonicalize()?, None));
    }
    let mut pieces = text.rsplitn(3, ':');
    let last = pieces.next().unwrap_or("");
    let rest = pieces.next();
    let earlier = pieces.next();
    if last.parse::<u32>().is_ok() {
        let (line, file) = if let Some(earlier) = earlier {
            if let Some(column) = rest.filter(|value| value.parse::<u32>().is_ok()) {
                (column.parse().ok(), earlier.to_owned())
            } else {
                (
                    last.parse().ok(),
                    format!("{}:{}", earlier, rest.unwrap_or("")),
                )
            }
        } else {
            (last.parse().ok(), rest.unwrap_or("").to_owned())
        };
        let p = target_path(&file, cwd);
        if p.is_file() {
            return Ok((p.canonicalize()?, line));
        }
    }
    anyhow::bail!("File not found: {text}")
}

pub fn existing_directory(mut path: PathBuf) -> PathBuf {
    while !path.is_dir() {
        if !path.pop() {
            return "/".into();
        }
    }
    path
}
