use anyhow::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
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
pub fn merge_stats(
    root: &Path,
    working: &[u8],
    staged: &[u8],
    changes: &[Change],
) -> std::collections::HashMap<PathBuf, (u32, u32)> {
    let mut stats = terminator_git::parse_numstat(root, staged);
    for (path, (added, deleted)) in terminator_git::parse_numstat(root, working) {
        let entry = stats.entry(path).or_insert((0, 0));
        entry.0 += added;
        entry.1 += deleted;
    }
    for change in changes.iter().filter(|change| change.status == "??") {
        let Ok(bytes) = fs::read(&change.path) else {
            continue;
        };
        if bytes.len() > 2 * 1024 * 1024 || bytes.contains(&0) {
            continue;
        }
        let mut lines = bytes.iter().filter(|byte| **byte == b'\n').count() as u32;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            lines += 1;
        }
        stats.entry(change.path.clone()).or_insert((0, 0)).0 += lines;
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
        kind: error
            .downcast_ref::<std::io::Error>()
            .map_or(std::io::ErrorKind::Other, std::io::Error::kind),
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
            entries.len() < 1000,
            "Directory exceeds the 1,000-entry display limit"
        );
        entries.push(Entry {
            ignored: entry.file_name() == ".git",
            path: entry.path(),
            directory: entry.file_type()?.is_dir(),
        });
    }
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
    Ok(entries)
}

#[cfg(test)]
pub fn context_cached(
    cwd: &Path,
    cache: &mut std::collections::HashMap<PathBuf, (PathBuf, Vec<PathBuf>)>,
) -> ContextData {
    let mut result = ContextData {
        cwd: cwd.into(),
        root: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: Default::default(),
        stats: Default::default(),
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
            Ok(v) => PathBuf::from(String::from_utf8_lossy(&v).trim()),
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
            if rest.unwrap_or("").parse::<u32>().is_ok() {
                (rest.unwrap().parse().ok(), earlier.to_owned())
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
pub async fn context_async(
    service: &crate::gui_services::Services,
    cwd: PathBuf,
    cancel: &terminator_core::async_service::CancellationToken,
) -> ContextData {
    let mut result = ContextData {
        cwd: cwd.clone(),
        root: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: Default::default(),
        stats: Default::default(),
        error: None,
    };
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let git = |root: PathBuf, args: Vec<std::ffi::OsString>| async move {
        terminator_git::run(service.processes(), service.fs(), &root, args, options()).await
    };
    let root = match git(
        cwd.clone(),
        vec!["rev-parse".into(), "--show-toplevel".into()],
    )
    .await
    {
        Ok(bytes) => PathBuf::from(String::from_utf8_lossy(&bytes).trim()),
        Err(_) => return result,
    };
    let root = service
        .fs()
        .run(cancel, move || Ok(root.canonicalize()?))
        .await
        .unwrap_or_else(|_| cwd.clone());
    for arg in ["--absolute-git-dir", "--git-common-dir"] {
        if let Ok(bytes) = git(root.clone(), vec!["rev-parse".into(), arg.into()]).await {
            let path = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
            result.git_dirs.push(if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            });
        }
    }
    result.root = Some(root.clone());
    result.branch = git(root.clone(), vec!["branch".into(), "--show-current".into()])
        .await
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    if result.branch.is_empty() {
        result.branch = "Detached HEAD".into();
    }
    match git(
        root.clone(),
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "-z".into(),
            "--untracked-files=all".into(),
        ],
    )
    .await
    {
        Ok(raw) => {
            result.changes = terminator_git::parse_porcelain(&root, &raw);
        }
        Err(error) => result.error = Some(error.to_string()),
    }
    result.decorations = terminator_git::decorations(&root, &result.changes);
    if !result.changes.is_empty() {
        let working = git(
            root.clone(),
            vec!["diff".into(), "--numstat".into(), "-z".into()],
        )
        .await
        .unwrap_or_default();
        let staged = git(
            root.clone(),
            vec![
                "diff".into(),
                "--cached".into(),
                "--numstat".into(),
                "-z".into(),
            ],
        )
        .await
        .unwrap_or_default();
        result.stats = merge_stats(&root, &working, &staged, &result.changes);
    }
    result
}

pub async fn directory_async(
    service: &crate::gui_services::Services,
    path: PathBuf,
    root: Option<PathBuf>,
    cancel: &terminator_core::async_service::CancellationToken,
) -> std::result::Result<Vec<Entry>, DirectoryError> {
    let result = async {
        let folder = path.clone();
        let operation = cancel.clone();
        let mut entries = service
            .fs()
            .run(cancel, move || entries_raw(&folder, &operation))
            .await?;
        if root.is_some() {
            let output = terminator_git::run(
                service.processes(),
                service.fs(),
                &path,
                terminator_git::ignore::CHECK_IGNORE_ARGV
                    .iter()
                    .map(std::ffi::OsString::from)
                    .collect(),
                CommandOptions {
                    input: Some(terminator_git::ignore::stdin_batch(&entries)),
                    stdout_limit: 4 * 1024 * 1024,
                    accepted_exit_codes: Some(vec![0, 1]),
                    ..Default::default()
                },
            )
            .await?;
            for entry in &mut entries {
                entry.ignored |= terminator_git::ignore::is_ignored(&output, &entry.path);
            }
        }
        Ok::<_, anyhow::Error>(entries)
    }
    .await;
    result.map_err(|error| DirectoryError {
        path,
        kind: error
            .downcast_ref::<std::io::Error>()
            .map_or(std::io::ErrorKind::Other, std::io::Error::kind),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_stats_adds_index_worktree_and_untracked_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let tracked = root.join("a.txt");
        fs::write(&tracked, "one\ntwo\nthree\n").unwrap();
        fs::write(root.join("new.txt"), "x\ny\n").unwrap();
        let changes = vec![
            Change {
                path: tracked.clone(),
                status: "MM".into(),
            },
            Change {
                path: root.join("new.txt"),
                status: "??".into(),
            },
        ];
        let stats = merge_stats(root, b"1\t1\ta.txt\0", b"3\t0\ta.txt\0", &changes);
        assert_eq!(stats[&tracked], (4, 1));
        assert_eq!(stats[&root.join("new.txt")], (2, 0));
    }

    #[test]
    fn reopening_a_project_through_a_path_alias_preserves_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("project");
        let alias = dir.path().join("old-location");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let projects = vec![terminator_core::Project {
            id: "original".into(),
            name: "Project".into(),
            path: alias,
            layout: serde_json::Value::Null,
        }];
        assert_eq!(
            project_for_directory(&projects, &real.canonicalize().unwrap())
                .unwrap()
                .id,
            "original"
        );
        assert!(project_for_directory(&projects, dir.path()).is_none());
    }

    #[test]
    fn untracked_descendants_get_individual_decorations() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        terminator_git::run_blocking(
            &root,
            &["init", "-q"],
            CommandOptions {
                stdout_limit: 4 * 1024 * 1024,
                ..Default::default()
            },
        )
        .unwrap();
        fs::create_dir_all(root.join("new/nested")).unwrap();
        fs::write(root.join("new/nested/space file.rs"), "").unwrap();
        let context = context_cached(&root, &mut Default::default());
        assert_eq!(context.changes.len(), 1);
        for relative in ["new", "new/nested", "new/nested/space file.rs"] {
            assert_eq!(context.decorations[&root.join(relative)], 'U');
        }
        assert_eq!(
            context.changes[0].letter(terminator_git::GitGroup::Untracked),
            'U'
        );
    }

    #[test]
    fn space_and_line_paths() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("space file.rs"), "hello").unwrap();
        let (p, line) = resolve_path("space file.rs:12:3", tmp.path()).unwrap();
        assert_eq!(p.file_name().unwrap(), "space file.rs");
        assert_eq!(line, Some(12));
        let target = resolve_target("space file.rs:12:3", tmp.path()).unwrap();
        assert!(matches!(target, Target::File(_, Some(12), Some(3))));
    }

    #[test]
    fn empty_and_failed_directory_reads_are_distinct() {
        let dir = tempfile::tempdir().unwrap();
        assert!(directory_result(dir.path(), false).unwrap().is_empty());
        let missing = dir.path().join("missing");
        let error = directory_result(&missing, false).unwrap_err();
        assert_eq!(error.path, missing);
        assert_eq!(error.kind, std::io::ErrorKind::NotFound);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn incomplete_listing_reports_its_limit_instead_of_returning_partial_success() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..1001 {
            fs::write(dir.path().join(n.to_string()), "").unwrap();
        }
        let error = directory_result(dir.path(), false).unwrap_err();
        assert!(error.message.contains("1,000-entry display limit"));
    }

    #[test]
    fn compact_path_replaces_home_prefix_with_tilde() {
        let home = Path::new("/Users/me");
        assert_eq!(
            compact_path_under(&home.join("docs/PANIC_AUDIT.md"), Some(home)),
            "~/docs/PANIC_AUDIT.md"
        );
        assert_eq!(compact_path_under(home, Some(home)), "~");
        assert_eq!(
            compact_path_under(Path::new("/tmp/terminator-paste.png"), Some(home)),
            "/tmp/terminator-paste.png"
        );
        assert_eq!(
            compact_path_under(Path::new("/Users/me-other/docs/a.md"), Some(home)),
            "/Users/me-other/docs/a.md"
        );
        assert_eq!(
            compact_path_under(Path::new("/Users/me/docs/a.md"), None),
            "/Users/me/docs/a.md"
        );
        assert_eq!(
            compact_path_under(Path::new("/Users/me/docs/a.md"), Some(Path::new(""))),
            "/Users/me/docs/a.md"
        );
    }

    #[test]
    fn target_compact_shortens_home_and_keeps_the_full_path_for_copy() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let path = home.join("docs/a.md");
        let target = Target::File(path.clone(), Some(12), Some(3));
        assert_eq!(target.compact(), "~/docs/a.md:12:3");
        assert_eq!(target.display(), format!("{}:12:3", path.display()));
        assert!(!Target::File(path, Some(12), None).display().contains('~'));
        let url = Target::Url("https://example.com/a".into());
        assert_eq!(url.compact(), url.display());
        assert_eq!(
            Target::File(PathBuf::from("/tmp/terminator-paste.png"), None, None).compact(),
            "/tmp/terminator-paste.png"
        );
    }
}
