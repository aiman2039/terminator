//! Apply to source: a task worktree's commits go into its source checkout,
//! as a merge of its branch or a cherry-pick of the range since the task
//! base. Only on an explicit click after a confirmation, with the quick
//! 20-second cap, never prompting. A dirty source checkout is refused
//! before Git runs; conflicts stay in the source checkout for a terminal.
//! Names never reach Git as options: the branch goes as its full ref name,
//! commits as full hashes.
use super::probe::git_out;
use std::path::{Path, PathBuf};
use std::time::Duration;
use terminator_core::{CommandOptions, ProcessError, git, run_command};

/// Cap of the merge or cherry-pick.
const APPLY_CAP: Duration = Duration::from_secs(20);

/// How the commits reach the source checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApplyMode {
    /// `git merge --no-edit refs/heads/<branch>`.
    Merge,
    /// `git cherry-pick <base>..<head>`.
    CherryPick,
}

impl ApplyMode {
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::CherryPick => "Cherry-pick",
        }
    }
}

/// One Apply to source.
#[derive(Clone, Debug)]
pub(crate) struct ApplyRequest {
    pub mode: ApplyMode,
    /// The task worktree.
    pub worktree: PathBuf,
    /// The source (main) checkout that receives the commits.
    pub source: PathBuf,
    /// The saved task base commit, if any.
    pub base: Option<String>,
}

/// Why the source checkout cannot take the commits now.
pub(crate) const SOURCE_DIRTY: &str =
    "The source checkout has uncommitted changes. Commit or shelve them first.";

fn full_hash(cwd: &Path, rev: &str) -> Option<String> {
    let hex = rev.len() >= 4 && rev.chars().all(|c| c.is_ascii_hexdigit());
    let spec = format!("{rev}^{{commit}}");
    let named = rev.starts_with("refs/heads/") || rev == "HEAD";
    (hex || named)
        .then(|| git_out(cwd, &["rev-parse", "--verify", "--quiet", &spec]))
        .flatten()
        .filter(|hash| !hash.is_empty())
}

/// Runs the merge or cherry-pick in the source checkout.
pub(crate) fn apply(request: &ApplyRequest) -> Result<String, String> {
    let source = &request.source;
    let dirty = git_out(source, &["status", "--porcelain", "--untracked-files=no"])
        .ok_or("Could not read the source checkout's status")?;
    if !dirty.is_empty() {
        return Err(SOURCE_DIRTY.into());
    }
    let head = full_hash(&request.worktree, "HEAD").ok_or("The worktree has no commit")?;
    match request.mode {
        ApplyMode::Merge => {
            let branch = git_out(&request.worktree, &["symbolic-ref", "-q", "HEAD"])
                .filter(|full| full.starts_with("refs/heads/"));
            // The branch by its full ref; a detached worktree by its hash.
            let target = branch.as_deref().unwrap_or(&head);
            run(source, ApplyMode::Merge, &["merge", "--no-edit", target])?;
            let name = branch
                .as_deref()
                .and_then(|full| full.strip_prefix("refs/heads/"))
                .unwrap_or("the worktree");
            Ok(format!("Merged {name} into the source checkout"))
        }
        ApplyMode::CherryPick => {
            let source_head = full_hash(source, "HEAD").ok_or("The source has no commit")?;
            let base = request
                .base
                .as_deref()
                .and_then(|base| full_hash(&request.worktree, base))
                .or_else(|| git_out(source, &["merge-base", &source_head, &head]))
                .ok_or("No task base to count the commits from")?;
            let range = format!("{base}..{head}");
            let count = git_out(source, &["rev-list", "--count", &range])
                .and_then(|count| count.parse::<usize>().ok())
                .unwrap_or(0);
            if count == 0 {
                return Err("The worktree has no commits since its task base".into());
            }
            run(source, ApplyMode::CherryPick, &["cherry-pick", &range])?;
            Ok(format!(
                "Cherry-picked {count} commit{} into the source checkout",
                if count == 1 { "" } else { "s" }
            ))
        }
    }
}

/// The Git call; a conflict says where to finish it.
fn run(source: &Path, mode: ApplyMode, args: &[&str]) -> Result<(), String> {
    let mut command = git::command(source, args);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    let options = CommandOptions {
        timeout: APPLY_CAP,
        ..Default::default()
    };
    let verb = mode.verb();
    match run_command(command, options) {
        Ok(_) => Ok(()),
        Err(ProcessError::TimedOut { .. }) => Err(format!(
            "{verb} stopped after {} seconds. Check the source checkout in a terminal",
            APPLY_CAP.as_secs()
        )),
        Err(ProcessError::Failed { stderr, .. }) => {
            Err(conflict(source, mode)
                .unwrap_or_else(|| format!("{verb} failed: {}", stderr.trim())))
        }
        Err(error) => Err(format!("{verb} failed: {error}")),
    }
}

/// A stopped merge or cherry-pick, left for a terminal.
fn conflict(source: &Path, mode: ApplyMode) -> Option<String> {
    let (marker, command) = match mode {
        ApplyMode::Merge => ("MERGE_HEAD", "merge"),
        ApplyMode::CherryPick => ("CHERRY_PICK_HEAD", "cherry-pick"),
    };
    git_out(source, &["rev-parse", "--verify", "--quiet", marker])?;
    Some(format!(
        "{} stopped on conflicts in {}. Resolve them in a terminal there, then run git {command} --continue (or git {command} --abort).",
        mode.verb(),
        source.display()
    ))
}
