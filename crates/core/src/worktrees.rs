//! Git remains the authority. These helpers add validation and a small registry
//! boundary; they never force removal, overwrite a checkout, or delete a branch.
use crate::{CommandOptions, run_command};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
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
    let mut command =
        Command::new(crate::find_executable("git").context("Git executable not found")?);
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
    let mut command =
        Command::new(crate::find_executable("git").context("Git executable not found")?);
    command
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let out = run_command(command, CommandOptions::default())?.stdout;
    let path = String::from_utf8(out).context("Git directory is not UTF-8")?;
    Ok(PathBuf::from(path.trim_end()).canonicalize()?)
}
pub fn resolve_start(cwd: &Path, reference: &str) -> Result<String> {
    ensure!(
        !reference.is_empty() && !reference.starts_with('-'),
        "Invalid starting revision"
    );
    let mut c = Command::new(crate::find_executable("git").context("Git not found")?);
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
        let field = std::str::from_utf8(part).context("Worktree path is not UTF-8")?;
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
pub fn add(common: &Path, path: &Path, branch: Option<&str>, start: &str) -> Result<GitWorktree> {
    let path = dunce::simplified(path);
    ensure!(path.is_absolute(), "Worktree path must be absolute");
    ensure!(path.to_str().is_some(), "Worktree path must be UTF-8");
    ensure!(!path.exists(), "Worktree destination already exists");
    ensure!(
        !start.starts_with('-') && !start.is_empty(),
        "Invalid starting revision"
    );
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
        ensure!(
            !branch.starts_with('-') && !branch.is_empty(),
            "Invalid branch name"
        );
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
        .context("Git created checkout but did not return it in worktree list")
}
pub fn remove(common: &Path, path: &Path) -> Result<()> {
    let path = dunce::simplified(path);
    let list = list(common)?;
    let record = list
        .iter()
        .find(|w| w.path == path)
        .context("Worktree no longer registered in Git")?;
    ensure!(
        !record.bare && !record.locked,
        "Cannot remove a bare or locked worktree"
    );
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
/// Project identity does not follow `cd`. Hooks can also be delayed or absent
/// in descendants, so check both recorded paths and owned process directories.
pub fn ensure_unused(path: &Path, project: &str, sessions: &[crate::Session]) -> Result<()> {
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System, UpdateKind};
    let path = path.canonicalize()?;
    let inside = |candidate: &Path| {
        candidate.starts_with(&path) || candidate.canonicalize().is_ok_and(|p| p.starts_with(&path))
    };
    let live: Vec<_> = sessions.iter().filter(|s| s.lifecycle.live()).collect();
    for session in &live {
        ensure!(
            session.project_id != project
                && !inside(&session.cwd)
                && !session.file.as_ref().is_some_and(|file| {
                    inside(&if file.is_absolute() {
                        file.clone()
                    } else {
                        session.cwd.join(file)
                    })
                }),
            "Stop sessions using this worktree before removing the checkout"
        );
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
        let pid = Pid::from_u32(
            session
                .pid
                .context("Cannot verify a live session's process")?,
        );
        let process = system
            .process(pid)
            .context("Live session process changed; retry removal")?;
        ensure!(
            process.start_time().abs_diff(session.created) <= 2,
            "Live session process identity changed; cannot safely remove checkout"
        );
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
            .context("Cannot inspect a live session process directory; retry after it exits")?;
        ensure!(
            !inside(cwd),
            "Stop processes using this worktree before removing the checkout"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn add_list_and_remove_preserve_dirty_checkouts_and_branch_refs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("file"), "base").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        // std canonicalization supplies a verbatim path on Windows. Git must
        // receive a compatible path even when callers supply that form.
        let common = common_dir(&repo).unwrap().canonicalize().unwrap();
        let path = dunce::canonicalize(dir.path())
            .unwrap()
            .join("space checkout");
        let requested = dir.path().canonicalize().unwrap().join("space checkout");
        let record = add(&common, &requested, Some("task-fixture"), "HEAD").unwrap();
        assert_eq!(record.path, path);
        assert_eq!(record.branch.as_deref(), Some("task-fixture"));
        assert_eq!(list(&common).unwrap().len(), 2);
        std::fs::write(path.join("untracked"), "keep").unwrap();
        assert!(remove(&common, &requested).is_err());
        assert!(path.join("untracked").exists());
        std::fs::remove_file(path.join("untracked")).unwrap();
        remove(&common, &requested).unwrap();
        assert!(!path.exists());
        git(&repo, &["show-ref", "--verify", "refs/heads/task-fixture"]);
        assert!(add(&common, &repo, Some("other"), "HEAD").is_err());
        assert!(add(&common, &path, Some("--bad"), "HEAD").is_err());
    }
    #[test]
    fn locked_clean_checkout_is_preserved_until_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("file"), "base").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        // std canonicalization supplies a verbatim path on Windows. Git must
        // receive a compatible path even when callers supply that form.
        let common = common_dir(&repo).unwrap().canonicalize().unwrap();
        let path = dunce::canonicalize(dir.path())
            .unwrap()
            .join("locked checkout");
        add(&common, &path, Some("locked-fixture"), "HEAD").unwrap();
        let head = resolve_start(&repo, "refs/heads/locked-fixture").unwrap();
        git(
            &repo,
            &[
                "worktree",
                "lock",
                "--reason",
                "fixture lock",
                path.to_str().unwrap(),
            ],
        );

        let error = remove(&common, &path).unwrap_err();
        assert!(error.to_string().contains("locked"), "{error:#}");
        assert_eq!(std::fs::read(path.join("file")).unwrap(), b"base");
        let records = list(&common).unwrap();
        assert_eq!(records.len(), 2);
        let retained = records.iter().find(|w| w.path == path).unwrap();
        assert!(retained.locked);
        assert_eq!(retained.branch.as_deref(), Some("locked-fixture"));
        assert_eq!(
            resolve_start(&repo, "refs/heads/locked-fixture").unwrap(),
            head
        );

        git(&repo, &["worktree", "unlock", path.to_str().unwrap()]);
        remove(&common, &path).unwrap();
        assert!(!path.exists());
        assert!(!list(&common).unwrap().iter().any(|w| w.path == path));
        assert_eq!(
            resolve_start(&repo, "refs/heads/locked-fixture").unwrap(),
            head
        );
    }
    #[test]
    fn porcelain_records_keep_spaces_and_lock_metadata() {
        let records=parse(b"worktree /a path\0HEAD 123\0branch refs/heads/topic\0locked reason\0\0worktree /bare\0bare\0\0").unwrap();
        assert_eq!(records[0].path, PathBuf::from("/a path"));
        assert!(records[0].locked);
        assert!(records[1].bare);
    }
}
