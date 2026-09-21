use anyhow::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
    pub ignored: bool,
}
#[derive(Clone, Debug)]
pub struct Change {
    pub path: PathBuf,
    pub status: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitGroup {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}
impl GitGroup {
    pub const ALL: [Self; 4] = [
        Self::Conflicts,
        Self::Staged,
        Self::Changes,
        Self::Untracked,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Staged => "Staged Changes",
            Self::Changes => "Changes",
            Self::Untracked => "Untracked",
        }
    }
}
impl Change {
    pub fn conflict(&self) -> bool {
        matches!(
            self.status.as_str(),
            "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU"
        )
    }
    /// Explorer precedence: conflicts, working tree, then index.
    pub fn decoration(&self) -> char {
        if self.conflict() {
            return '!';
        }
        if self.status == "??" {
            return 'U';
        }
        let bytes = self.status.as_bytes();
        if bytes.len() != 2 {
            return ' ';
        }
        if bytes[1] != b' ' {
            bytes[1] as char
        } else {
            bytes[0] as char
        }
    }
    pub fn in_group(&self, group: GitGroup) -> bool {
        let bytes = self.status.as_bytes();
        if bytes.len() != 2 {
            return false;
        }
        match group {
            GitGroup::Conflicts => self.conflict(),
            GitGroup::Staged => !self.conflict() && !matches!(bytes[0], b' ' | b'?' | b'!'),
            GitGroup::Changes => !self.conflict() && !matches!(bytes[1], b' ' | b'?' | b'!'),
            GitGroup::Untracked => self.status == "??",
        }
    }
    pub fn letter(&self, group: GitGroup) -> char {
        if self.conflict() {
            '!'
        } else if self.status == "??" {
            'U'
        } else if group == GitGroup::Staged {
            self.status.chars().next().unwrap_or(' ')
        } else {
            self.status.chars().nth(1).unwrap_or(' ')
        }
    }
    /// Working-tree side wins when both exist. Conflicts are not auto-reviewed.
    #[cfg(test)]
    pub fn default_staged(&self) -> Option<bool> {
        if self.conflict() {
            None
        } else if self.in_group(GitGroup::Changes) || self.in_group(GitGroup::Untracked) {
            Some(false)
        } else if self.in_group(GitGroup::Staged) {
            Some(true)
        } else {
            None
        }
    }
}
#[derive(Clone, Debug)]
pub struct ContextData {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub git_dirs: Vec<PathBuf>,
    pub branch: String,
    pub changes: Vec<Change>,
    pub decorations: std::collections::HashMap<PathBuf, char>,
    pub error: Option<String>,
}
#[cfg(test)]
pub fn entries(path: &Path) -> Result<Vec<Entry>> {
    entries_known(path, git(path, &["rev-parse", "--show-toplevel"]).is_ok())
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
    entries_known(path, in_git).map_err(|error| DirectoryError {
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
pub fn entries_known(path: &Path, in_git: bool) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
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
    // Ask Git so nested ignore files, negations, global excludes and tracked
    // files use the same rules as the user's repository. Keep dotfiles visible.
    if in_git {
        use std::os::unix::ffi::OsStrExt;
        for batch in entries.chunks_mut(1000) {
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(path)
                .args(["check-ignore", "--stdin", "-z"]);
            let mut input = Vec::new();
            for entry in batch.iter() {
                input.extend_from_slice(entry.path.as_os_str().as_bytes());
                input.push(0);
            }
            let ignored = run_status(command, true, Some(input))?;
            for entry in batch {
                entry.ignored |= ignored
                    .split(|b| *b == 0)
                    .any(|p| p == entry.path.as_os_str().as_bytes());
            }
        }
    }
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
    Ok(entries)
}
#[cfg(test)]
pub fn run(cmd: Command) -> Result<Vec<u8>> {
    run_status(cmd, false, None)
}
#[cfg(test)]
fn run_status(cmd: Command, allow_no_matches: bool, input: Option<Vec<u8>>) -> Result<Vec<u8>> {
    Ok(terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            input,
            stdout_limit: 4 * 1024 * 1024,
            accepted_exit_codes: Some(if allow_no_matches {
                vec![0, 1]
            } else {
                vec![0]
            }),
            ..Default::default()
        },
    )?
    .stdout)
}
#[cfg(test)]
fn git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut c = Command::new("git");
    c.env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(cwd)
        .args(args);
    run(c)
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
        error: None,
    };
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
            let mut parts = raw.split(|b| *b == 0);
            while let Some(part) = parts.next() {
                if part.len() < 4 {
                    continue;
                }
                let status = String::from_utf8_lossy(&part[..2]).into_owned();
                use std::os::unix::ffi::OsStrExt;
                let path = root.join(std::ffi::OsStr::from_bytes(&part[3..]));
                if status.contains('R') || status.contains('C') {
                    let _ = parts.next();
                }
                result.changes.push(Change { path, status });
            }
        }
        Err(e) => result.error = Some(e.to_string()),
    }
    result.decorations = decorations(&root, &result.changes);
    result
}
/// Build folder and file lookups once on the refresh worker. The total ordering
/// makes folder badges independent of Git output order.
fn decorations(root: &Path, changes: &[Change]) -> std::collections::HashMap<PathBuf, char> {
    let mut result = std::collections::HashMap::new();
    for change in changes {
        let status = change.decoration();
        for path in change.path.ancestors().take_while(|p| p.starts_with(root)) {
            let entry = result.entry(path.to_path_buf()).or_insert(status);
            if decoration_priority(status) > decoration_priority(*entry) {
                *entry = status;
            }
        }
    }
    result
}
fn decoration_priority(status: char) -> u8 {
    match status {
        '!' => 8,
        'D' => 7,
        'M' => 6,
        'R' => 5,
        'C' => 4,
        'A' => 3,
        'T' => 2,
        'U' => 1,
        _ => 0,
    }
}
pub fn status_description(status: char) -> &'static str {
    match status {
        '!' => "Conflict",
        'D' => "Deleted",
        'M' => "Modified",
        'R' => "Renamed",
        'C' => "Copied",
        'A' => "Added",
        'T' => "Type changed",
        'U' => "Untracked",
        _ => "Unchanged",
    }
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
        error: None,
    };
    let git = |root: PathBuf, args: Vec<std::ffi::OsString>| async move {
        crate::diff::git_async(service, &root, args).await
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
            let mut parts = raw.split(|b| *b == 0);
            while let Some(part) = parts.next() {
                if part.len() < 4 {
                    continue;
                }
                let status = String::from_utf8_lossy(&part[..2]).into_owned();
                use std::os::unix::ffi::OsStrExt;
                let path = root.join(std::ffi::OsStr::from_bytes(&part[3..]));
                if status.contains('R') || status.contains('C') {
                    let _ = parts.next();
                }
                result.changes.push(Change { path, status });
            }
        }
        Err(error) => result.error = Some(error.to_string()),
    }
    result.decorations = decorations(&root, &result.changes);
    result
}

pub async fn directory_async(
    service: &crate::gui_services::Services,
    path: PathBuf,
    root: Option<PathBuf>,
    cancel: &terminator_core::async_service::CancellationToken,
) -> std::result::Result<Vec<Entry>, DirectoryError> {
    use std::os::unix::ffi::OsStrExt;
    let result = async {
        let folder = path.clone();
        let operation = cancel.clone();
        let mut entries = service
            .fs()
            .run(cancel, move || entries_raw(&folder, &operation))
            .await?;
        if let Some(root) = root {
            let mut command = Command::new("git");
            command
                .env("GIT_OPTIONAL_LOCKS", "0")
                .arg("-C")
                .arg(&path)
                .args(["check-ignore", "--stdin", "-z"]);
            let mut input = Vec::new();
            for entry in &entries {
                input.extend_from_slice(entry.path.as_os_str().as_bytes());
                input.push(0);
            }
            let key = service
                .fs()
                .run(cancel, move || {
                    Ok(terminator_core::async_process::git_key(&root))
                })
                .await?;
            let output = service
                .processes()
                .run(
                    command,
                    terminator_core::CommandOptions {
                        input: Some(input),
                        stdout_limit: 4 * 1024 * 1024,
                        accepted_exit_codes: Some(vec![0, 1]),
                        ..Default::default()
                    },
                    Some(key),
                )
                .await?;
            for entry in &mut entries {
                entry.ignored |= output
                    .stdout
                    .split(|b| *b == 0)
                    .any(|p| p == entry.path.as_os_str().as_bytes());
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
    fn explorer_status_precedence_and_folder_aggregation_are_deterministic() {
        for (status, expected) in [
            ("??", 'U'),
            ("AM", 'M'),
            ("MD", 'D'),
            ("M ", 'M'),
            ("R ", 'R'),
            ("UU", '!'),
            ("AA", '!'),
            ("DU", '!'),
        ] {
            assert_eq!(
                Change {
                    path: "/repo/a".into(),
                    status: status.into()
                }
                .decoration(),
                expected
            );
        }
        let mut changes = vec![
            Change {
                path: "/repo/new/file".into(),
                status: "??".into(),
            },
            Change {
                path: "/repo/new/deleted".into(),
                status: " D".into(),
            },
            Change {
                path: "/repo/conflict".into(),
                status: "UU".into(),
            },
        ];
        let first = decorations(Path::new("/repo"), &changes);
        changes.reverse();
        assert_eq!(first, decorations(Path::new("/repo"), &changes));
        assert_eq!(first[Path::new("/repo")], '!');
        assert_eq!(first[Path::new("/repo/new")], 'D');
        assert_eq!(first[Path::new("/repo/new/file")], 'U');
        assert!(!first.contains_key(Path::new("/")));
    }
    #[test]
    fn untracked_descendants_get_individual_decorations() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]).unwrap();
        fs::create_dir_all(root.join("new/nested")).unwrap();
        fs::write(root.join("new/nested/space file.rs"), "").unwrap();
        let context = context_cached(&root, &mut Default::default());
        assert_eq!(context.changes.len(), 1);
        for relative in ["new", "new/nested", "new/nested/space file.rs"] {
            assert_eq!(context.decorations[&root.join(relative)], 'U');
        }
        assert_eq!(context.changes[0].letter(GitGroup::Untracked), 'U');
    }
    #[test]
    fn partially_staged_and_conflicts_are_grouped_correctly() {
        let change = Change {
            path: "file".into(),
            status: "MM".into(),
        };
        assert!(change.in_group(GitGroup::Staged));
        assert!(change.in_group(GitGroup::Changes));
        assert!(!change.in_group(GitGroup::Conflicts));
        for status in ["DD", "AU", "UD", "UA", "DU", "AA", "UU"] {
            let change = Change {
                path: "file".into(),
                status: status.into(),
            };
            assert!(change.in_group(GitGroup::Conflicts));
            assert!(!change.in_group(GitGroup::Staged));
            assert!(!change.in_group(GitGroup::Changes));
        }
        assert!(
            Change {
                path: "file".into(),
                status: "??".into()
            }
            .in_group(GitGroup::Untracked)
        );
    }
    #[test]
    fn default_staged_prefers_the_working_tree() {
        let working = Change {
            path: "a.rs".into(),
            status: "MM".into(),
        };
        assert_eq!(working.default_staged(), Some(false));
        let staged_only = Change {
            path: "a.rs".into(),
            status: "M ".into(),
        };
        assert_eq!(staged_only.default_staged(), Some(true));
        let untracked = Change {
            path: "a.rs".into(),
            status: "??".into(),
        };
        assert_eq!(untracked.default_staged(), Some(false));
        let conflict = Change {
            path: "a.rs".into(),
            status: "UU".into(),
        };
        assert_eq!(conflict.default_staged(), None);
    }
    #[test]
    fn partially_staged_diffs_show_the_correct_side() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]).unwrap();
        git(root, &["config", "user.name", "Fixture"]).unwrap();
        git(root, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        let path = root.join("space file.txt");
        fs::write(&path, "base\n").unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-qm", "fixture"]).unwrap();
        fs::write(&path, "staged\n").unwrap();
        git(root, &["add", "."]).unwrap();
        fs::write(&path, "working\n").unwrap();
        let context = context_cached(root, &mut Default::default());
        assert_eq!(context.changes[0].status, "MM");
        let staged = crate::diff::document(crate::diff::DiffRequest {
            cwd: root,
            path: &path,
            staged: true,
        })
        .unwrap();
        let working = crate::diff::document(crate::diff::DiffRequest {
            cwd: root,
            path: &path,
            staged: false,
        })
        .unwrap();
        let text = |doc: &crate::diff::DiffDocument| {
            doc.unified
                .iter()
                .flat_map(|line| line.spans.iter().map(|s| s.text.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text(&staged).contains("staged"));
        assert!(text(&working).contains("working"));
    }
    #[test]
    fn git_ignore_rules_preserve_dotfiles_negations_and_tracked_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]).unwrap();
        fs::write(root.join("tracked.log"), "tracked").unwrap();
        git(root, &["add", "tracked.log"]).unwrap();
        fs::write(root.join(".gitignore"), "*.log\n!keep.log\nbuild/\n").unwrap();
        for name in ["ignored.log", "keep.log", ".visible"] {
            fs::write(root.join(name), "").unwrap();
        }
        fs::create_dir(root.join("build")).unwrap();
        let entries = entries(root).unwrap();
        let ignored = |name: &str| {
            entries
                .iter()
                .find(|e| e.path.file_name().unwrap() == name)
                .unwrap()
                .ignored
        };
        assert!(ignored("ignored.log"));
        assert!(ignored("build"));
        assert!(ignored(".git"));
        assert!(!ignored(".visible"));
        assert!(!ignored(".gitignore"));
        assert!(!ignored("keep.log"));
        assert!(!ignored("tracked.log"));
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
