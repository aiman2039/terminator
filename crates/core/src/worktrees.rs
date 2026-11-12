//! Git remains the authority. These helpers add validation and a small registry
//! boundary; they never force removal, overwrite a checkout, or delete a branch.
use crate::{CommandOptions, run_command};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Why a worktree could not be listed, added, or safely removed.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("Git executable not found")]
    GitExecutableNotFound,
    #[error("Git not found")]
    GitNotFound,
    #[error("Git directory is not UTF-8")]
    CommonDirNotUtf8 {
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("Worktree path is not UTF-8")]
    ListNotUtf8 {
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("Invalid starting revision")]
    InvalidStart { revision: String },
    #[error("Worktree path must be absolute")]
    RelativePath { path: PathBuf },
    #[error("Worktree path must be UTF-8")]
    PathNotUtf8 { path: PathBuf },
    #[error("Worktree destination already exists")]
    DestinationExists { path: PathBuf },
    #[error("Invalid branch name")]
    InvalidBranch { branch: String },
    #[error("Git created checkout but did not return it in worktree list")]
    MissingAfterAdd { path: PathBuf },
    #[error("Worktree no longer registered in Git")]
    Unregistered { path: PathBuf },
    #[error("Cannot remove a bare or locked worktree")]
    BareOrLocked { path: PathBuf },
    #[error(
        "Worktree has ignored files ({first}); Git would delete them. Move or delete them first"
    )]
    IgnoredFiles { path: PathBuf, first: String },
    #[error("Stop sessions using this worktree before removing the checkout")]
    SessionInside { session: String },
    #[error("Cannot verify a live session's process")]
    NoSessionPid { session: String },
    #[error("Live session process changed; retry removal")]
    SessionProcessGone { session: String },
    #[error("Live session process identity changed; cannot safely remove checkout")]
    SessionIdentityChanged { session: String },
    #[error("Cannot inspect a live session process directory; retry after it exits")]
    ProcessCwdUnknown { pid: u32 },
    #[error("Stop processes using this worktree before removing the checkout")]
    ProcessInside { pid: u32 },
    #[error(transparent)]
    Process(#[from] crate::ProcessError),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
type Result<T, E = WorktreeError> = std::result::Result<T, E>;

fn git_executable() -> Result<PathBuf> {
    crate::find_executable("git").ok_or(WorktreeError::GitExecutableNotFound)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitWorktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
    pub project_id: String,
    pub common_dir: PathBuf,
    pub path: PathBuf,
    pub created: u64,
    pub removed: bool,
}
fn run(common: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let mut command = Command::new(git_executable()?);
    command
        .arg("--git-dir")
        .arg(dunce::simplified(common))
        .args(args);
    Ok(run_command(
        command,
        CommandOptions {
            timeout: std::time::Duration::from_secs(30),
            ..Default::default()
        },
    )?
    .stdout)
}
pub fn common_dir(cwd: &Path) -> Result<PathBuf> {
    let mut command = Command::new(git_executable()?);
    command
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let out = run_command(command, CommandOptions::default())?.stdout;
    let path =
        String::from_utf8(out).map_err(|source| WorktreeError::CommonDirNotUtf8 { source })?;
    Ok(PathBuf::from(path.trim_end()).canonicalize()?)
}
pub fn resolve_start(cwd: &Path, reference: &str) -> Result<String> {
    if reference.is_empty() || reference.starts_with('-') {
        return Err(WorktreeError::InvalidStart {
            revision: reference.to_owned(),
        });
    }
    let mut c = Command::new(crate::find_executable("git").ok_or(WorktreeError::GitNotFound)?);
    c.arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--verify", "--end-of-options"])
        .arg(format!("{reference}^{{commit}}"));
    Ok(
        String::from_utf8(run_command(c, CommandOptions::default())?.stdout)?
            .trim()
            .into(),
    )
}
pub fn list(common: &Path) -> Result<Vec<GitWorktree>> {
    let bytes = run(
        common,
        &[
            "worktree".as_ref(),
            "list".as_ref(),
            "--porcelain".as_ref(),
            "-z".as_ref(),
        ],
    )?;
    parse(&bytes)
}
fn parse(bytes: &[u8]) -> Result<Vec<GitWorktree>> {
    let mut result = Vec::new();
    let mut record = None::<GitWorktree>;
    for part in bytes.split(|b| *b == 0) {
        if part.is_empty() {
            if let Some(record) = record.take() {
                result.push(record);
            }
            continue;
        }
        let field =
            std::str::from_utf8(part).map_err(|source| WorktreeError::ListNotUtf8 { source })?;
        if let Some(path) = field.strip_prefix("worktree ") {
            record = Some(GitWorktree {
                path: path.into(),
                head: None,
                branch: None,
                bare: false,
                locked: false,
                prunable: false,
            });
        } else if let Some(record) = &mut record {
            if let Some(head) = field.strip_prefix("HEAD ") {
                record.head = Some(head.into());
            }
            if let Some(branch) = field.strip_prefix("branch ") {
                record.branch = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).into());
            }
            record.bare |= field == "bare";
            record.locked |= field == "locked" || field.starts_with("locked ");
            record.prunable |= field == "prunable" || field.starts_with("prunable ");
        }
    }
    if let Some(record) = record {
        result.push(record);
    }
    Ok(result)
}
pub struct AddInput<'a> {
    pub common: &'a Path,
    pub path: &'a Path,
    pub branch: Option<&'a str>,
    pub start: &'a str,
}

pub fn add(input: AddInput<'_>) -> Result<GitWorktree> {
    let AddInput {
        common,
        path,
        branch,
        start,
    } = input;
    let path = dunce::simplified(path);
    if !path.is_absolute() {
        return Err(WorktreeError::RelativePath { path: path.into() });
    }
    if path.to_str().is_none() {
        return Err(WorktreeError::PathNotUtf8 { path: path.into() });
    }
    if path.exists() {
        return Err(WorktreeError::DestinationExists { path: path.into() });
    }
    if start.starts_with('-') || start.is_empty() {
        return Err(WorktreeError::InvalidStart {
            revision: start.to_owned(),
        });
    }
    let revision = format!("{start}^{{commit}}");
    run(
        common,
        &[
            "rev-parse".as_ref(),
            "--verify".as_ref(),
            "--end-of-options".as_ref(),
            revision.as_ref(),
        ],
    )?;
    let mut args = vec!["worktree".as_ref(), "add".as_ref()];
    if let Some(branch) = branch {
        if branch.starts_with('-') || branch.is_empty() {
            return Err(WorktreeError::InvalidBranch {
                branch: branch.to_owned(),
            });
        }
        run(
            common,
            &[
                "check-ref-format".as_ref(),
                "--branch".as_ref(),
                branch.as_ref(),
            ],
        )?;
        args.push("-b".as_ref());
        args.push(branch.as_ref());
    } else {
        args.push("--detach".as_ref());
    }
    args.extend(["--".as_ref(), path.as_os_str(), start.as_ref()]);
    run(common, &args)?;
    let canonical = dunce::canonicalize(path)?;
    list(common)?
        .into_iter()
        .find(|w| w.path == canonical)
        .ok_or(WorktreeError::MissingAfterAdd { path: canonical })
}
/// The linked checkout Git already has at `path` on `branch` (detached
/// when `None`), unlocked and present: what an earlier add created before
/// it failed to finish. The main checkout never counts.
pub fn existing(common: &Path, path: &Path, branch: Option<&str>) -> Result<Option<GitWorktree>> {
    let Ok(canonical) = dunce::canonicalize(path) else {
        return Ok(None);
    };
    Ok(list(common)?.into_iter().skip(1).find(|w| {
        w.path == canonical && !w.bare && !w.locked && !w.prunable && w.branch.as_deref() == branch
    }))
}
pub fn remove(common: &Path, path: &Path) -> Result<()> {
    let path = dunce::simplified(path);
    let list = list(common)?;
    let record = list
        .iter()
        .find(|w| w.path == path)
        .ok_or_else(|| WorktreeError::Unregistered { path: path.into() })?;
    if record.bare || record.locked {
        return Err(WorktreeError::BareOrLocked { path: path.into() });
    }
    // A checkout folder the user already deleted has nothing left to lose;
    // `git worktree remove` (without --force) then only drops the registration.
    if !folder_missing(path)? {
        ensure_no_ignored(path)?;
    }
    run(
        common,
        &[
            "worktree".as_ref(),
            "remove".as_ref(),
            "--".as_ref(),
            path.as_os_str(),
        ],
    )?;
    Ok(())
}
fn folder_missing(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}
/// `git worktree remove` refuses dirty and untracked content but deletes
/// ignored files (`.env`, local databases) silently, so refuse those too.
fn ensure_no_ignored(path: &Path) -> Result<()> {
    let mut command = Command::new(git_executable()?);
    command.arg("-C").arg(path).args([
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ]);
    let options = CommandOptions {
        timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    };
    let output = run_command(command, options)?.stdout;
    match output.split(|b| *b == 0).find(|name| !name.is_empty()) {
        Some(first) => Err(WorktreeError::IgnoredFiles {
            path: path.into(),
            first: String::from_utf8_lossy(first).into_owned(),
        }),
        None => Ok(()),
    }
}
/// Project identity does not follow `cd`. Hooks can also be delayed or absent
/// in descendants, so check both recorded paths and owned process directories.
pub fn ensure_unused(path: &Path, project: &str, sessions: &[crate::Session]) -> Result<()> {
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System, UpdateKind};
    // A deleted checkout cannot be canonicalized; compare the recorded path.
    let path = if folder_missing(path)? {
        path.to_path_buf()
    } else {
        path.canonicalize()?
    };
    let inside = |candidate: &Path| {
        candidate.starts_with(&path) || candidate.canonicalize().is_ok_and(|p| p.starts_with(&path))
    };
    let live: Vec<_> = sessions.iter().filter(|s| s.lifecycle.live()).collect();
    for session in &live {
        let uses = session.project_id == project
            || inside(&session.cwd)
            || session.file.as_ref().is_some_and(|file| {
                inside(&if file.is_absolute() {
                    file.clone()
                } else {
                    session.cwd.join(file)
                })
            });
        if uses {
            return Err(WorktreeError::SessionInside {
                session: session.id.clone(),
            });
        }
    }
    if live.is_empty() {
        return Ok(());
    }
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always)),
    );
    let mut owned = std::collections::HashSet::new();
    for session in live {
        let pid = Pid::from_u32(session.pid.ok_or_else(|| WorktreeError::NoSessionPid {
            session: session.id.clone(),
        })?);
        let process = system
            .process(pid)
            .ok_or_else(|| WorktreeError::SessionProcessGone {
                session: session.id.clone(),
            })?;
        if process.start_time().abs_diff(session.created) > 2 {
            return Err(WorktreeError::SessionIdentityChanged {
                session: session.id.clone(),
            });
        }
        owned.insert(pid);
    }
    loop {
        let previous = owned.len();
        for (pid, process) in system.processes() {
            if process.parent().is_some_and(|p| owned.contains(&p)) {
                owned.insert(*pid);
            }
        }
        if owned.len() == previous {
            break;
        }
    }
    for pid in owned {
        let cwd = system
            .process(pid)
            .and_then(|p| p.cwd())
            .ok_or(WorktreeError::ProcessCwdUnknown { pid: pid.as_u32() })?;
        if inside(cwd) {
            return Err(WorktreeError::ProcessInside { pid: pid.as_u32() });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "worktrees_tests.rs"]
mod tests;
