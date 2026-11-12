//! Branch actions of the Git panel and the Commit Log: new/rename/delete
//! branch, Merge, Rebase, and the commit actions Checkout, Cherry-pick and
//! Revert. Each runs only on an explicit click, as a quick call with the
//! 20-second cap, and never prompts. Fetch, Pull and Push run as their own
//! cancellable job (`sync.rs`). Typed
//! names never reach Git as options: a new name passes `git check-ref-format`,
//! an existing branch goes as its full ref name, a commit as its full hash.
use super::*;
use terminator_core::ProcessError;

/// One branch or sync action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchOp {
    Fetch,
    Pull,
    /// Push the current branch without a commit (sets the upstream on
    /// `origin` when there is none). Never forced.
    Push,
    /// Create `name` at `start` (a commit hash or branch; `HEAD` when
    /// `None`) and switch to it.
    New {
        name: String,
        start: Option<String>,
    },
    Rename {
        from: String,
        to: String,
    },
    /// `git branch -d`; `force` (`-D`) only after a second explicit choice.
    Delete {
        name: String,
        force: bool,
    },
    /// Merge this branch into the current one.
    Merge(String),
    /// Rebase the current branch onto this branch.
    Rebase(String),
    /// Switch to this commit, detached.
    Checkout(String),
    CherryPick(String),
    Revert(String),
}

/// Why a delete stopped: the branch has commits that no other branch has.
pub(crate) const NOT_MERGED: &str = "not fully merged";

impl BranchOp {
    /// The status-bar name of a failure.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::New { .. } => "New branch",
            Self::Rename { .. } => "Rename branch",
            Self::Delete { .. } => "Delete branch",
            Self::Merge(_) => "Merge",
            Self::Rebase(_) => "Rebase",
            Self::Checkout(_) => "Checkout revision",
            Self::CherryPick(_) => "Cherry-pick",
            Self::Revert(_) => "Revert",
        }
    }
}

/// Runs `op` in `root`.
pub(super) fn perform(root: &Path, op: &BranchOp) -> Result<Report> {
    let name = op.name();
    let message = match op {
        BranchOp::Fetch | BranchOp::Pull | BranchOp::Push => {
            bail!("{name} runs as a cancellable sync job")
        }
        BranchOp::New { name: new, start } => {
            let new = new_name(root, new)?;
            let mut args = vec!["switch", "-c", new.as_str()];
            let start = start.as_deref().map(|rev| target(root, rev)).transpose()?;
            if let Some(start) = &start {
                args.push(start);
            }
            run(root, name, &args)?;
            format!("Created and switched to {new}")
        }
        BranchOp::Rename { from, to } => {
            let old = local_branch(root, from)?;
            let new = new_name(root, to)?;
            run(root, name, &["branch", "-m", &old, &new])?;
            format!("Renamed {from} to {new}")
        }
        BranchOp::Delete {
            name: branch,
            force,
        } => {
            local_branch(root, branch)?;
            let flag = if *force { "-D" } else { "-d" };
            run(root, name, &["branch", flag, "--", branch])?;
            format!("Deleted {branch}")
        }
        BranchOp::Merge(branch) => {
            let full = branch_ref(root, branch)?;
            run(root, name, &["merge", "--no-edit", &full])?;
            format!("Merged {branch}")
        }
        BranchOp::Rebase(branch) => {
            let full = branch_ref(root, branch)?;
            run(root, name, &["rebase", &full])?;
            format!("Rebased onto {branch}")
        }
        BranchOp::Checkout(rev) => {
            let hash = commit(root, rev)?;
            run(root, name, &["switch", "--detach", &hash])?;
            format!("Checked out {}", short(&hash))
        }
        BranchOp::CherryPick(rev) => {
            let hash = commit(root, rev)?;
            run(root, name, &["cherry-pick", &hash])?;
            format!("Cherry-picked {}", short(&hash))
        }
        BranchOp::Revert(rev) => {
            let hash = commit(root, rev)?;
            run(root, name, &["revert", "--no-edit", &hash])?;
            format!("Reverted {}", short(&hash))
        }
    };
    Ok(Report::Message(message))
}

fn short(hash: &str) -> &str {
    hash.get(..7).unwrap_or(hash)
}

/// One Git call with the quick cap. Git never asks for a password or an
/// editor; a call that hits the cap says so and names the terminal.
fn run(root: &Path, name: &str, args: &[&str]) -> Result<String> {
    let mut command = git::command(root, args);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    match run_command(command, git_options()) {
        Ok(output) => Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned()),
        Err(ProcessError::TimedOut { .. }) => bail!(
            "{name} stopped after {} seconds. Run git {} in a terminal for a slow remote",
            QUICK_GIT_TIMEOUT.as_secs(),
            args.first().copied().unwrap_or_default()
        ),
        Err(ProcessError::Failed { stderr, .. }) => {
            bail!("{name} failed: {}", stderr.trim())
        }
        Err(error) => Err(error.into()),
    }
}

/// A typed name that looks like an option or has spaces is refused.
fn plain(text: &str) -> Result<&str> {
    let text = text.trim();
    ensure!(!text.is_empty(), "Enter a name");
    ensure!(
        !text.starts_with('-') && !text.chars().any(|c| c.is_whitespace() || c.is_control()),
        "Names cannot start with '-' or contain spaces"
    );
    Ok(text)
}

/// A new branch name as `git check-ref-format --branch` accepts it, and
/// not one that exists.
pub(super) fn new_name(root: &Path, text: &str) -> Result<String> {
    let text = plain(text)?;
    let checked = git_text(root, &["check-ref-format", "--branch", text])
        .ok()
        .filter(|name| name == text)
        .with_context(|| format!("{text} is not a valid branch name"))?;
    ensure!(
        !exists(root, &format!("refs/heads/{checked}")),
        "Branch {checked} already exists"
    );
    Ok(checked)
}

fn exists(root: &Path, full: &str) -> bool {
    git_text(root, &["rev-parse", "--verify", "--quiet", full]).is_ok_and(|out| !out.is_empty())
}

/// The full ref name of a local branch.
fn local_branch(root: &Path, name: &str) -> Result<String> {
    let name = plain(name)?;
    let full = format!("refs/heads/{name}");
    ensure!(exists(root, &full), "No local branch {name}");
    Ok(full.trim_start_matches("refs/heads/").to_owned())
}

/// The full ref name of a local, else a remote-tracking, branch.
pub(super) fn branch_ref(root: &Path, name: &str) -> Result<String> {
    let name = plain(name)?;
    ["refs/heads/", "refs/remotes/"]
        .iter()
        .map(|prefix| format!("{prefix}{name}"))
        .find(|full| exists(root, full))
        .with_context(|| format!("No branch {name}"))
}

/// The full hash of a commit named by a hash.
pub(super) fn commit(root: &Path, text: &str) -> Result<String> {
    let text = plain(text)?;
    ensure!(
        (4..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit()),
        "Not a commit hash: {text}"
    );
    let hash = git_text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{text}^{{commit}}"),
        ],
    )
    .ok()
    .filter(|hash| !hash.is_empty())
    .with_context(|| format!("No commit {text}"))?;
    Ok(hash)
}

/// A start point: a commit hash, or a branch as its full ref name.
fn target(root: &Path, text: &str) -> Result<String> {
    commit(root, text).or_else(|_| branch_ref(root, text))
}

#[cfg(test)]
#[path = "branch_tests.rs"]
mod tests;
