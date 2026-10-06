//! Explicit file and Git actions from the sidebar. Diff review stays read-only.
//! Every path has to stay inside the repository or project root.

use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::{CommandOptions, git, os_name, run_command};

#[derive(Clone, Debug)]
pub enum Op {
    Stage(PathBuf),
    Unstage(PathBuf),
    /// Revert a tracked worktree file, or delete an untracked file.
    Discard {
        path: PathBuf,
        untracked: bool,
    },
    Delete(PathBuf),
    Commit(String),
    Branches,
    Switch(String),
    Log,
    Compare {
        base: Option<String>,
    },
    CreateFile(PathBuf),
    CreateDir(PathBuf),
    Duplicate(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Reveal(PathBuf),
}

#[derive(Clone, Debug)]
pub enum Report {
    Message(String),
    Branches(Vec<String>),
    Log(Vec<(String, String)>),
    Compare(CompareData),
}

/// Branch comparison for the Git sidebar: upstream tracking, ahead/behind,
/// the base ref used for "committed on branch", and the files it changed.
#[derive(Clone, Debug, Default)]
pub struct CompareData {
    pub root: Option<PathBuf>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub base: Option<String>,
    pub files: Vec<CommittedFile>,
}

#[derive(Clone, Debug)]
pub struct CommittedFile {
    pub path: PathBuf,
    pub letter: char,
    pub added: u32,
    pub deleted: u32,
}

pub fn perform(root: &Path, op: Op) -> Result<Report> {
    match op {
        Op::Stage(path) => git_path(root, &["add", "--"], &path, "Staged"),
        Op::Unstage(path) => git_path(root, &["restore", "--staged", "--"], &path, "Unstaged"),
        Op::Discard { path, untracked } => {
            if untracked {
                let path = contained_file(root, &path)?;
                fs::remove_file(&path).with_context(|| format!("delete {}", path.display()))?;
                Ok(Report::Message(format!("Removed {}", path.display())))
            } else {
                // Restore the worktree from the index (the default source),
                // never from HEAD: partially staged files keep their staged
                // content, and newly staged files have no HEAD version.
                git_path(root, &["restore", "--worktree", "--"], &path, "Discarded")
            }
        }
        Op::Delete(path) => delete(root, &path),
        Op::Commit(message) => commit(root, &message),
        Op::Branches => Ok(Report::Branches(branches(root)?)),
        Op::Switch(name) => switch(root, &name),
        Op::Log => Ok(Report::Log(log(root)?)),
        Op::Compare { base } => Ok(Report::Compare(compare(root, base.as_deref())?)),
        Op::CreateFile(path) => {
            let path = new_path(root, &path)?;
            ensure!(!link_exists(&path), "{} already exists", path.display());
            fs::write(&path, "").with_context(|| format!("create {}", path.display()))?;
            Ok(Report::Message(format!("Created {}", path.display())))
        }
        Op::CreateDir(path) => {
            let path = new_path(root, &path)?;
            ensure!(!link_exists(&path), "{} already exists", path.display());
            fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
            Ok(Report::Message(format!("Created {}", path.display())))
        }
        Op::Duplicate(path) => {
            let path = contained_file(root, &path)?;
            let dest = duplicate_destination(&path)?;
            if fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                replicate_symlink(&path, &dest)?;
            } else {
                fs::copy(&path, &dest).with_context(|| format!("copy {}", path.display()))?;
            }
            Ok(Report::Message(format!("Copied to {}", dest.display())))
        }
        Op::Rename { from, to } => {
            let from = contained_file(root, &from)?;
            let to = new_path(root, &to)?;
            ensure!(!link_exists(&to), "{} already exists", to.display());
            fs::rename(&from, &to).with_context(|| format!("rename {}", from.display()))?;
            Ok(Report::Message(format!("Renamed to {}", to.display())))
        }
        Op::Reveal(path) => {
            let path = contained_join(root, &path, true)?;
            reveal(&path)?;
            Ok(Report::Message(format!("Revealed {}", path.display())))
        }
    }
}

#[derive(Clone, Debug)]
pub enum NamePrompt {
    File { dir: PathBuf, name: String },
    Folder { dir: PathBuf, name: String },
    Rename { from: PathBuf, name: String },
}

pub fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn git_options() -> CommandOptions {
    CommandOptions {
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    }
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let output = run_command(git::command(root, args), git_options())?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn git_path(root: &Path, prefix: &[&str], path: &Path, verb: &str) -> Result<Report> {
    let relative = relative_arg(root, path)?;
    let mut args: Vec<&str> = prefix.to_vec();
    args.push(&relative);
    git_text(root, &args)?;
    Ok(Report::Message(format!("{verb} {relative}")))
}

fn commit(root: &Path, message: &str) -> Result<Report> {
    let message = message.trim();
    ensure!(!message.is_empty(), "Commit message is empty");
    ensure!(
        !message.contains('\0'),
        "Commit message contains a null byte"
    );
    let mut options = git_options();
    options.input = Some(message.as_bytes().to_vec());
    run_command(git::command(root, &["commit", "-F", "-"]), options)?;
    Ok(Report::Message("Committed".into()))
}

fn branches(root: &Path) -> Result<Vec<String>> {
    let text = git_text(root, &["branch", "--format=%(refname:short)"])?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn switch(root: &Path, name: &str) -> Result<Report> {
    ensure!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
            && !name.starts_with('-')
            && !name.split('/').any(|part| part == ".." || part.is_empty()),
        "Branch name is not allowed"
    );
    git_text(root, &["switch", "--", name])?;
    Ok(Report::Message(format!("Switched to {name}")))
}

fn log(root: &Path) -> Result<Vec<(String, String)>> {
    let text = git_text(root, &["log", "-n", "40", "--pretty=format:%h%x09%s"])?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once('\t')?;
            Some((hash.to_owned(), subject.to_owned()))
        })
        .collect())
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    Ok(run_command(git::command(root, args), git_options())?.stdout)
}

/// A base ref must not look like a flag and must resolve to a commit.
fn valid_ref(root: &Path, base: &str) -> bool {
    !base.is_empty()
        && !base.starts_with('-')
        && !base.contains("..")
        && !base.chars().any(char::is_whitespace)
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | '@'))
        && git_text(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{base}^{{commit}}"),
            ],
        )
        .is_ok_and(|out| !out.is_empty())
}

/// Parse `git diff --name-status -z`. Rename/copy records carry `old\0new`; the
/// destination path wins.
fn parse_name_status(root: &Path, raw: &[u8]) -> Vec<(PathBuf, char)> {
    let mut result = Vec::new();
    let mut parts = raw.split(|b| *b == 0).filter(|part| !part.is_empty());
    while let Some(status) = parts.next() {
        let letter = *status.first().unwrap_or(&b' ') as char;
        let Some(first) = parts.next() else {
            break;
        };
        let path = if matches!(letter, 'R' | 'C') {
            match parts.next() {
                Some(new) => new,
                None => first,
            }
        } else {
            first
        };
        result.push((root.join(os_name(path)), letter));
    }
    result
}

fn compare(root: &Path, base_override: Option<&str>) -> Result<CompareData> {
    let mut data = CompareData {
        root: Some(root.to_path_buf()),
        ..Default::default()
    };
    if let Ok(upstream) = git_text(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    ) && !upstream.is_empty()
    {
        data.upstream = Some(upstream);
    }
    if let Some(upstream) = &data.upstream
        && let Ok(counts) = git_text(
            root,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{upstream}...HEAD"),
            ],
        )
    {
        let mut parts = counts.split_whitespace();
        data.behind = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        data.ahead = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    let base = base_override
        .map(str::to_owned)
        .or_else(|| data.upstream.clone());
    let Some(base) = base.filter(|base| valid_ref(root, base)) else {
        return Ok(data);
    };
    let range = format!("{base}...HEAD");
    let name_status = git_bytes(root, &["diff", "--name-status", "-z", &range]).unwrap_or_default();
    let numstat = git_bytes(root, &["diff", "--numstat", "-z", &range]).unwrap_or_default();
    let mut stats = terminator_git::parse_numstat(root, &numstat);
    data.base = Some(base);
    for (path, letter) in parse_name_status(root, &name_status) {
        let (added, deleted) = stats.remove(&path).unwrap_or((0, 0));
        data.files.push(CommittedFile {
            path,
            letter,
            added,
            deleted,
        });
    }
    Ok(data)
}

fn delete(root: &Path, path: &Path) -> Result<Report> {
    let path = contained_file(root, path)?;
    let tracked = relative_arg(root, &path).ok();
    if let Some(relative) = tracked.as_deref()
        && git_text(root, &["ls-files", "--error-unmatch", "--", relative]).is_ok()
    {
        git_text(root, &["rm", "-f", "--", relative])?;
    } else {
        fs::remove_file(&path).with_context(|| format!("delete {}", path.display()))?;
    }
    Ok(Report::Message(format!("Deleted {}", path.display())))
}

fn reveal(path: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("reveal file")?;
    Ok(())
}

fn relative_arg(root: &Path, path: &Path) -> Result<String> {
    let resolved = contained_link(root, path)?;
    let root = root.canonicalize().context("repository root")?;
    let relative = resolved
        .strip_prefix(&root)
        .context("path is outside the repository")?;
    Ok(relative.display().to_string())
}

/// Resolve `path` inside `root` without following its final component, so
/// symlink actions affect the link itself rather than its target.
/// Intermediate components resolve through the parent, which also keeps
/// missing worktree files (staged deletions) and broken links usable.
fn contained_join(root: &Path, path: &Path, allow_root: bool) -> Result<PathBuf> {
    let root = root.canonicalize().context("root")?;
    let name = path.file_name().context("missing file name")?;
    let parent = path.parent().context("missing parent")?;
    let parent = if parent.as_os_str().is_empty() {
        std::env::current_dir().context("current dir")?
    } else {
        parent.canonicalize().context("parent")?
    };
    ensure!(parent.starts_with(&root), "path escapes the root");
    let resolved = parent.join(name);
    ensure!(resolved.starts_with(&root), "path escapes the root");
    if !allow_root {
        ensure!(resolved != root, "refusing the repository root");
    }
    Ok(resolved)
}

fn contained_link(root: &Path, path: &Path) -> Result<PathBuf> {
    contained_join(root, path, false)
}

fn contained_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_link(root, path)?;
    let meta = fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    ensure!(
        meta.is_file() || meta.file_type().is_symlink(),
        "{} is not a file",
        path.display()
    );
    Ok(path)
}

/// `Path::exists` follows links and misses broken ones; the guards need the
/// link itself to exist.
fn link_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(unix)]
fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    let target = fs::read_link(path).with_context(|| format!("read link {}", path.display()))?;
    std::os::unix::fs::symlink(&target, dest)
        .with_context(|| format!("copy {}", path.display()))?;
    Ok(())
}

#[cfg(windows)]
fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    let target = fs::read_link(path).with_context(|| format!("read link {}", path.display()))?;
    let context = || format!("copy {}", path.display());
    if path.canonicalize().is_ok_and(|resolved| resolved.is_dir()) {
        std::os::windows::fs::symlink_dir(&target, dest).with_context(context)?;
    } else {
        std::os::windows::fs::symlink_file(&target, dest).with_context(context)?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    fs::copy(path, dest).with_context(|| format!("copy {}", path.display()))?;
    Ok(())
}

fn new_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("missing file name")?
        .to_string_lossy();
    ensure!(
        !name.is_empty() && !name.contains('/') && name != "." && name != "..",
        "file name is not allowed"
    );
    let parent = path.parent().context("missing parent")?;
    let parent = if parent.exists() {
        parent.canonicalize().context("parent")?
    } else {
        bail!("parent directory does not exist")
    };
    let root = root.canonicalize().context("root")?;
    ensure!(parent.starts_with(&root), "path escapes the root");
    Ok(parent.join(name.as_ref()))
}

fn duplicate_destination(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().context("missing parent")?;
    let stem = path
        .file_stem()
        .context("missing name")?
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    for n in 1..100 {
        let name = if n == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {n}")
        };
        let file = match &ext {
            Some(ext) => format!("{name}.{ext}"),
            None => name,
        };
        let dest = parent.join(file);
        if !link_exists(&dest) {
            return Ok(dest);
        }
    }
    bail!("no free copy name")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        run_command(git::command(root, args), git_options()).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "ops@example.com"]);
        git(dir.path(), &["config", "user.name", "Ops"]);
        fs::write(dir.path().join("a.txt"), "base\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn stage_commit_and_discard_stay_inside_the_repo() {
        let dir = repo();
        let root = dir.path();
        let path = root.join("a.txt");
        fs::write(&path, "next\n").unwrap();
        perform(root, Op::Stage(path.clone())).unwrap();
        fs::write(root.join("new.txt"), "new\n").unwrap();
        perform(root, Op::Stage(root.join("new.txt"))).unwrap();
        perform(root, Op::Commit("next\n".into())).unwrap();
        fs::write(&path, "dirty\n").unwrap();
        perform(
            root,
            Op::Discard {
                path: path.clone(),
                untracked: false,
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "next\n");
        fs::write(root.join("scratch.txt"), "x").unwrap();
        perform(
            root,
            Op::Discard {
                path: root.join("scratch.txt"),
                untracked: true,
            },
        )
        .unwrap();
        assert!(!root.join("scratch.txt").exists());
        let outside = dir.path().join("../nope.txt");
        assert!(perform(root, Op::Stage(outside)).is_err());
    }

    #[test]
    fn name_status_rename_uses_the_destination_path() {
        let root = Path::new("/repo");
        let parsed = parse_name_status(root, b"R100\0old.rs\0new.rs\0M\0keep.rs\0");
        assert_eq!(
            parsed,
            vec![(root.join("new.rs"), 'R'), (root.join("keep.rs"), 'M'),]
        );
    }

    #[test]
    fn base_refs_reject_flags_ranges_and_missing_refs() {
        let dir = repo();
        let root = dir.path();
        assert!(valid_ref(root, "main"));
        assert!(!valid_ref(root, "--all"));
        assert!(!valid_ref(root, "a..b"));
        assert!(!valid_ref(root, "missing-ref"));
    }

    #[test]
    fn compare_without_an_upstream_reports_root_only() {
        let dir = repo();
        let data = compare(dir.path(), None).unwrap();
        assert_eq!(data.root.as_deref(), Some(dir.path()));
        assert!(data.upstream.is_none());
        assert!(data.base.is_none());
        assert!(data.files.is_empty());
    }

    #[test]
    fn switch_lists_branches_and_rejects_escape() {
        let dir = repo();
        let root = dir.path();
        git(root, &["branch", "feature"]);
        let branches = branches(root).unwrap();
        assert!(branches.iter().any(|name| name == "feature"));
        perform(root, Op::Switch("feature".into())).unwrap();
        assert!(perform(root, Op::Switch("--orphan".into())).is_err());
        let log = log(root).unwrap();
        assert_eq!(log[0].1, "base");
    }

    #[test]
    fn discard_restores_the_index_not_head() {
        let dir = repo();
        let root = dir.path();
        let path = root.join("a.txt");
        fs::write(&path, "staged\n").unwrap();
        git(root, &["add", "a.txt"]);
        fs::write(&path, "dirty\n").unwrap();
        perform(
            root,
            Op::Discard {
                path: path.clone(),
                untracked: false,
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "staged\n");
        let staged = git_text(root, &["diff", "--cached", "--", "a.txt"]).unwrap();
        assert!(staged.contains("+staged"), "staged hunk kept: {staged}");
    }

    #[test]
    fn discard_restores_a_newly_staged_file_from_the_index() {
        let dir = repo();
        let root = dir.path();
        let path = root.join("new.txt");
        fs::write(&path, "v1\n").unwrap();
        git(root, &["add", "new.txt"]);
        fs::write(&path, "v2\n").unwrap();
        perform(
            root,
            Op::Discard {
                path: path.clone(),
                untracked: false,
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1\n");
    }

    #[test]
    #[cfg(unix)]
    fn symlink_actions_affect_the_link_not_the_target() {
        let dir = repo();
        let root = dir.path();
        let target = root.join("target.txt");
        fs::write(&target, "data\n").unwrap();
        std::os::unix::fs::symlink("target.txt", root.join("link.txt")).unwrap();
        perform(root, Op::Stage(root.join("link.txt"))).unwrap();
        let files = git_text(root, &["ls-files"]).unwrap();
        assert!(
            files.lines().any(|line| line == "link.txt"),
            "staged the link: {files}"
        );
        perform(
            root,
            Op::Rename {
                from: root.join("link.txt"),
                to: root.join("moved.txt"),
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "data\n");
        assert!(
            fs::symlink_metadata(root.join("moved.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        perform(root, Op::Duplicate(root.join("moved.txt"))).unwrap();
        let copy = root.join("moved copy.txt");
        assert!(
            fs::symlink_metadata(&copy)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        perform(root, Op::Delete(copy)).unwrap();
        perform(root, Op::Delete(root.join("moved.txt"))).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "data\n");
        std::os::unix::fs::symlink("missing.txt", root.join("broken.txt")).unwrap();
        perform(root, Op::Delete(root.join("broken.txt"))).unwrap();
        assert!(fs::symlink_metadata(root.join("broken.txt")).is_err());
    }
}
