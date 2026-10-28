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
    Perforce(crate::perforce::Op),
    Stage(PathBuf),
    Unstage(PathBuf),
    /// Revert a tracked worktree file, or delete an untracked file.
    Discard {
        path: PathBuf,
        untracked: bool,
    },
    Delete(PathBuf),
    Commit {
        message: String,
        amend: bool,
        signoff: bool,
        push: bool,
    },
    /// Shelve staged paths with `git stash push --staged`.
    Shelve(Vec<PathBuf>),
    Shelves,
    Unshelve(String),
    DropShelf(String),
    Branches,
    /// The full message of `HEAD`, for Amend.
    HeadMessage,
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
    /// Move files or folders inside the root into `to_dir` (Explorer cut
    /// and paste, drag and drop). An existing name is an error.
    Move {
        from: Vec<PathBuf>,
        to_dir: PathBuf,
    },
    /// Copy files or folders into `to_dir` inside the root. Sources may
    /// live anywhere (a Finder drop); clashing names get a `copy` suffix.
    Copy {
        from: Vec<PathBuf>,
        to_dir: PathBuf,
    },
    Reveal(PathBuf),
}

#[derive(Clone, Debug)]
pub enum Report {
    Message(String),
    Branches(Vec<String>),
    HeadMessage(String),
    Shelves(Vec<(String, String)>),
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
        Op::Perforce(_) => bail!("Perforce actions require the async process service"),
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
        Op::Commit {
            message,
            amend,
            signoff,
            push,
        } => commit(CommitInput {
            root,
            message: &message,
            amend,
            signoff,
            push,
        }),
        Op::Shelve(paths) => shelve(root, &paths),
        Op::Shelves => Ok(Report::Shelves(shelves(root)?)),
        Op::Unshelve(reference) => {
            valid_shelf_ref(&reference)?;
            git_text(root, &["stash", "pop", "--quiet", &reference])?;
            Ok(Report::Message("Unshelved".into()))
        }
        Op::DropShelf(reference) => {
            valid_shelf_ref(&reference)?;
            git_text(root, &["stash", "drop", "--quiet", &reference])?;
            Ok(Report::Message("Shelf dropped".into()))
        }
        Op::Branches => Ok(Report::Branches(branches(root)?)),
        Op::HeadMessage => Ok(Report::HeadMessage(git_text(
            root,
            &["log", "-1", "--format=%B"],
        )?)),
        Op::Switch(name) => switch(root, &name),
        Op::Log => Ok(Report::Log(log(root)?)),
        Op::Compare { base } => Ok(Report::Compare(compare(root, base.as_deref())?)),
        Op::CreateFile(path) => {
            let path = new_nested_path(root, &path)?;
            ensure!(!link_exists(&path), "{} already exists", path.display());
            fs::write(&path, "").with_context(|| format!("create {}", path.display()))?;
            Ok(Report::Message(format!("Created {}", path.display())))
        }
        Op::CreateDir(path) => {
            let path = new_nested_path(root, &path)?;
            ensure!(!link_exists(&path), "{} already exists", path.display());
            fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
            Ok(Report::Message(format!("Created {}", path.display())))
        }
        Op::Duplicate(path) => {
            let path = contained_entry(root, &path)?;
            let dest = duplicate_destination(&path)?;
            copy_entry(&path, &dest)?;
            Ok(Report::Message(format!("Copied to {}", dest.display())))
        }
        Op::Move { from, to_dir } => {
            let dir = contained_dir(root, &to_dir)?;
            for source in &from {
                let source = contained_entry(root, source)?;
                ensure!(
                    !dir.starts_with(&source),
                    "cannot move {} into itself",
                    source.display()
                );
                let dest = dir.join(source.file_name().context("missing file name")?);
                if dest == source {
                    continue;
                }
                ensure!(!link_exists(&dest), "{} already exists", dest.display());
                fs::rename(&source, &dest).with_context(|| format!("move {}", source.display()))?;
            }
            Ok(Report::Message(format!(
                "Moved {} item{} to {}",
                from.len(),
                if from.len() == 1 { "" } else { "s" },
                dir.display()
            )))
        }
        Op::Copy { from, to_dir } => {
            let dir = contained_dir(root, &to_dir)?;
            for source in &from {
                let meta = fs::symlink_metadata(source)
                    .with_context(|| format!("stat {}", source.display()))?;
                ensure!(
                    !meta.is_dir() || !dir.starts_with(source),
                    "cannot copy {} into itself",
                    source.display()
                );
                let name = source.file_name().context("missing file name")?;
                let dest = if link_exists(&dir.join(name)) {
                    duplicate_destination(&dir.join(name))?
                } else {
                    dir.join(name)
                };
                copy_entry(source, &dest)?;
            }
            Ok(Report::Message(format!(
                "Copied {} item{} to {}",
                from.len(),
                if from.len() == 1 { "" } else { "s" },
                dir.display()
            )))
        }
        Op::Rename { from, to } => {
            let from = contained_entry(root, &from)?;
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

/// Error prefix for a push that failed after its commit succeeded; the draft
/// must not be restored then, or committing again would duplicate the commit.
pub(crate) const PUSH_FAILED: &str = "Committed, but push failed";

struct CommitInput<'a> {
    root: &'a Path,
    message: &'a str,
    amend: bool,
    signoff: bool,
    push: bool,
}

fn commit(input: CommitInput<'_>) -> Result<Report> {
    let CommitInput {
        root,
        message,
        amend,
        signoff,
        push,
    } = input;

    let message = message.trim();
    ensure!(amend || !message.is_empty(), "Commit message is empty");
    ensure!(
        !message.contains('\0'),
        "Commit message contains a null byte"
    );
    // Amending with no message keeps the previous message untouched.
    let mut args: Vec<&str> = vec!["commit"];
    if amend {
        args.push("--amend");
    }
    if signoff {
        args.push("--signoff");
    }
    let mut options = git_options();
    if amend && message.is_empty() {
        args.push("--no-edit");
    } else {
        args.extend(["-F", "-"]);
        options.input = Some(message.as_bytes().to_vec());
    }
    run_command(git::command(root, &args), options)?;
    if push {
        git_text(root, &["push"]).context(PUSH_FAILED)?;
        return Ok(Report::Message("Committed and pushed".into()));
    }
    Ok(Report::Message("Committed".into()))
}

/// Only `stash@{N}` references may reach `git stash pop/drop`.
fn valid_shelf_ref(reference: &str) -> Result<()> {
    let inner = reference
        .strip_prefix("stash@{")
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or_default();
    ensure!(
        !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()),
        "Invalid shelf reference"
    );
    Ok(())
}

fn shelve(root: &Path, paths: &[PathBuf]) -> Result<Report> {
    ensure!(!paths.is_empty(), "Nothing staged to shelve");
    let mut relatives = Vec::new();
    for path in paths {
        relatives.push(relative_arg(root, path)?);
    }
    let mut args: Vec<&str> = vec![
        "stash",
        "push",
        "--staged",
        "-m",
        "Shelve (Terminator)",
        "--",
    ];
    args.extend(relatives.iter().map(String::as_str));
    git_text(root, &args)?;
    Ok(Report::Message("Shelved staged changes".into()))
}

fn shelves(root: &Path) -> Result<Vec<(String, String)>> {
    let text = git_text(root, &["stash", "list", "--format=%gd%x09%gs"])?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let (reference, message) = line.split_once('\t')?;
            Some((reference.to_owned(), message.to_owned()))
        })
        .collect())
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

/// Explorer delete touches only the working tree (as in `IntelliJ`): the
/// index is left alone, so a tracked file shows as deleted, unstaged.
fn delete(root: &Path, path: &Path) -> Result<Report> {
    let path = contained_entry(root, path)?;
    move_to_trash(&path)?;
    Ok(Report::Message(format!(
        "Moved {} to Trash",
        path.display()
    )))
}

/// Recoverable removal: the system Trash on macOS and through `gio` on
/// Linux. Tests remove directly so they never fill the user's Trash.
#[cfg(all(target_os = "macos", not(test)))]
fn move_to_trash(path: &Path) -> Result<()> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| anyhow::anyhow!("{}", error.localizedDescription()))
        .with_context(|| format!("move {} to Trash", path.display()))
}

#[cfg(all(target_os = "linux", not(test)))]
fn move_to_trash(path: &Path) -> Result<()> {
    let status = std::process::Command::new("gio")
        .arg("trash")
        .arg("--")
        .arg(path)
        .status()
        .with_context(|| format!("move {} to Trash (gio)", path.display()))?;
    ensure!(status.success(), "gio trash failed for {}", path.display());
    Ok(())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
fn move_to_trash(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("delete {}", path.display()))
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

/// A file, folder, or link inside the root (never the root itself).
fn contained_entry(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_link(root, path)?;
    fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    Ok(path)
}

/// An existing directory inside the root (the root itself allowed).
fn contained_dir(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_join(root, path, true)?;
    ensure!(path.is_dir(), "{} is not a folder", path.display());
    Ok(path)
}

/// Copy a file, link, or folder tree. Links are recreated, never followed,
/// so a copy cannot pull in files from outside the source.
fn copy_entry(source: &Path, dest: &Path) -> Result<()> {
    let meta =
        fs::symlink_metadata(source).with_context(|| format!("stat {}", source.display()))?;
    if meta.file_type().is_symlink() {
        return replicate_symlink(source, dest);
    }
    if meta.is_dir() {
        fs::create_dir(dest).with_context(|| format!("create {}", dest.display()))?;
        for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
            let entry = entry?;
            copy_entry(&entry.path(), &dest.join(entry.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(source, dest).with_context(|| format!("copy {}", source.display()))?;
    Ok(())
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

/// Like [`new_path`], but a typed `a/b/c.rs` creates the missing `a/b`
/// first (as `IntelliJ` New File does). Only plain names may follow the deepest
/// existing ancestor, and that ancestor must resolve inside the root, so
/// neither `..` nor a symlinked directory can escape it.
fn new_nested_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let mut existing = path.parent().context("missing parent")?;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().context("missing parent")?;
        missing.push(name.to_owned());
        existing = existing.parent().context("missing parent")?;
    }
    let base = existing.canonicalize().context("parent")?;
    let root = root.canonicalize().context("root")?;
    ensure!(base.starts_with(&root), "path escapes the root");
    let mut parent = base;
    for name in missing.iter().rev() {
        let text = name.to_string_lossy();
        ensure!(
            !text.is_empty() && text != "." && text != "..",
            "folder name is not allowed"
        );
        parent.push(name);
    }
    fs::create_dir_all(&parent).with_context(|| format!("create {}", parent.display()))?;
    new_path(
        &root,
        &parent.join(path.file_name().context("missing file name")?),
    )
}

fn duplicate_destination(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().context("missing parent")?;
    // Folders keep their whole name: `v1.2 copy`, not `v1 copy.2`.
    let folder = path.is_dir();
    let stem = if folder {
        path.file_name()
    } else {
        path.file_stem()
    }
    .context("missing name")?
    .to_string_lossy()
    .into_owned();
    let ext = path
        .extension()
        .filter(|_| !folder)
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
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        git(dir.path(), &["config", "user.email", "ops@example.com"]);
        git(dir.path(), &["config", "user.name", "Ops"]);
        fs::write(dir.path().join("a.txt"), "base\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn new_file_with_a_nested_name_creates_its_folders() {
        let dir = repo();
        let root = dir.path();
        perform(root, Op::CreateFile(root.join("src/deep/mod.rs"))).unwrap();
        assert!(root.join("src/deep/mod.rs").is_file());
        perform(root, Op::CreateDir(root.join("docs/guide/img"))).unwrap();
        assert!(root.join("docs/guide/img").is_dir());
        assert!(perform(root, Op::CreateFile(root.join("x/../../escape.rs"))).is_err());
        assert!(!root.parent().unwrap().join("escape.rs").exists());
    }

    #[test]
    fn head_message_returns_the_full_last_message() {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("a.txt"), "next\n").unwrap();
        git(root, &["commit", "-qam", "subject\n\nbody line"]);
        let Ok(Report::HeadMessage(message)) = perform(root, Op::HeadMessage) else {
            panic!("expected the HEAD message");
        };
        assert_eq!(message, "subject\n\nbody line");
    }

    #[test]
    fn folders_rename_duplicate_and_delete() {
        let dir = repo();
        let root = dir.path();
        fs::create_dir_all(root.join("pkg/sub")).unwrap();
        fs::write(root.join("pkg/sub/x.rs"), "x").unwrap();
        perform(root, Op::Duplicate(root.join("pkg"))).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("pkg copy/sub/x.rs")).unwrap(),
            "x"
        );
        perform(
            root,
            Op::Rename {
                from: root.join("pkg copy"),
                to: root.join("lib"),
            },
        )
        .unwrap();
        assert!(root.join("lib/sub/x.rs").is_file());
        perform(root, Op::Delete(root.join("lib"))).unwrap();
        assert!(!root.join("lib").exists());
        // The root itself is never a delete target.
        assert!(perform(root, Op::Delete(root.to_path_buf())).is_err());
    }

    #[test]
    fn move_and_copy_into_folders() {
        let dir = repo();
        let root = dir.path();
        fs::create_dir(root.join("dest")).unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        perform(
            root,
            Op::Move {
                from: vec![root.join("b.txt")],
                to_dir: root.join("dest"),
            },
        )
        .unwrap();
        assert!(root.join("dest/b.txt").is_file());
        assert!(!root.join("b.txt").exists());
        // A folder cannot move into its own subtree.
        assert!(
            perform(
                root,
                Op::Move {
                    from: vec![root.join("dest")],
                    to_dir: root.join("dest"),
                },
            )
            .is_err()
        );
        // Copies from outside the root (a Finder drop) get a free name.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("b.txt"), "outside").unwrap();
        perform(
            root,
            Op::Copy {
                from: vec![outside.path().join("b.txt")],
                to_dir: root.join("dest"),
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(root.join("dest/b.txt")).unwrap(), "b");
        assert_eq!(
            fs::read_to_string(root.join("dest/b copy.txt")).unwrap(),
            "outside"
        );
        // Destinations stay inside the root.
        assert!(
            perform(
                root,
                Op::Copy {
                    from: vec![root.join("a.txt")],
                    to_dir: outside.path().to_path_buf(),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn delete_leaves_the_index_alone() {
        let dir = repo();
        let root = dir.path();
        perform(root, Op::Delete(root.join("a.txt"))).unwrap();
        assert!(!root.join("a.txt").exists());
        let output = run_command(
            git::command(root, &["status", "--porcelain"]),
            git_options(),
        )
        .unwrap();
        // Unstaged deletion (` D`), not a staged `git rm` (`D `).
        assert_eq!(String::from_utf8(output.stdout).unwrap(), " D a.txt\n");
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
        perform(
            root,
            Op::Commit {
                message: "next\n".into(),
                amend: false,
                signoff: false,
                push: false,
            },
        )
        .unwrap();
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
    fn amend_signoff_and_push_commit_variants() {
        let dir = repo();
        let root = dir.path();
        let subject = |root: &Path| git_text(root, &["log", "-1", "--pretty=%s"]).unwrap();
        let body = |root: &Path| git_text(root, &["log", "-1", "--pretty=%b"]).unwrap();
        // Amend with an empty message keeps the previous subject.
        fs::write(root.join("a.txt"), "amended\n").unwrap();
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
        perform(
            root,
            Op::Commit {
                message: String::new(),
                amend: true,
                signoff: true,
                push: false,
            },
        )
        .unwrap();
        assert_eq!(subject(root), "base");
        assert!(body(root).contains("Signed-off-by: Ops <ops@example.com>"));
        // Amend with a message replaces the subject.
        fs::write(root.join("a.txt"), "again\n").unwrap();
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
        perform(
            root,
            Op::Commit {
                message: "renamed".into(),
                amend: true,
                signoff: false,
                push: false,
            },
        )
        .unwrap();
        assert_eq!(subject(root), "renamed");
        // Push goes to a bare remote configured as upstream.
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
        let remote_arg = remote.path().to_string_lossy().into_owned();
        git(root, &["remote", "add", "origin", &remote_arg]);
        git(root, &["push", "-q", "-u", "origin", "main"]);
        fs::write(root.join("a.txt"), "pushed\n").unwrap();
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
        let report = perform(
            root,
            Op::Commit {
                message: "pushed change".into(),
                amend: false,
                signoff: false,
                push: true,
            },
        )
        .unwrap();
        assert!(matches!(report, Report::Message(ref m) if m == "Committed and pushed"));
        let remote_head = git_text(remote.path(), &["rev-parse", "main"]).unwrap();
        let local_head = git_text(root, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(remote_head, local_head);
    }

    #[test]
    fn shelve_unshelve_and_drop_roundtrip() {
        let dir = repo();
        let root = dir.path();
        assert!(valid_shelf_ref("stash@{0}").is_ok());
        assert!(valid_shelf_ref("stash@{12}").is_ok());
        assert!(valid_shelf_ref("--all").is_err());
        assert!(valid_shelf_ref("stash@{x}").is_err());
        assert!(perform(root, Op::Shelve(vec![])).is_err());
        fs::write(root.join("a.txt"), "shelved\n").unwrap();
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
        perform(root, Op::Shelve(vec![root.join("a.txt")])).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
        let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
            panic!("shelves report");
        };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "stash@{0}");
        assert!(list[0].1.contains("Shelve (Terminator)"));
        perform(root, Op::Unshelve("stash@{0}".into())).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "shelved\n");
        let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
            panic!("shelves report");
        };
        assert!(list.is_empty());
        // Drop removes a shelf without touching the worktree.
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
        perform(root, Op::Shelve(vec![root.join("a.txt")])).unwrap();
        perform(root, Op::DropShelf("stash@{0}".into())).unwrap();
        let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
            panic!("shelves report");
        };
        assert!(list.is_empty());
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
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
