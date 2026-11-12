//! Explicit file and Git actions from the sidebar. Diff review stays read-only.
//! Every path has to stay inside the repository or project root.
mod branch;
mod commit;
#[cfg(test)]
mod commit_tests;
mod entries;
mod pr;
#[cfg(test)]
mod tests;
mod vcs;

pub use branch::BranchOp;
pub(crate) use branch::NOT_MERGED;
#[cfg(test)]
pub(crate) use commit::{COMMIT_CANCELLED, PUSH_CANCELLED, PUSH_FAILED};
pub(crate) use commit::{
    COMMIT_RUNNING, COMMIT_WAIT, CommitRequest, CommitRisk, CommitRun, commit_cancelled,
    commit_landed, push_failed,
};
use entries::*;
use pr::create_pr;
pub(crate) use pr::review_cli;
use vcs::*;

use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::async_service::CancellationToken;
use terminator_core::{CommandOptions, git, os_name, run_command};

#[derive(Clone, Debug)]
pub enum Op {
    Perforce(crate::perforce::Op),
    Stage(PathBuf),
    /// Stage many paths with one `git add` (Stage all).
    StageAll(Vec<PathBuf>),
    Unstage(PathBuf),
    /// Revert a tracked worktree file, or delete an untracked file.
    Discard {
        path: PathBuf,
        untracked: bool,
    },
    Delete(PathBuf),
    /// `git commit`, then `git push` when `push` is set. Hooks and the
    /// network may need minutes: it runs as a cancellable job with a long
    /// cap, and `cancel` kills Git's process group.
    Commit {
        message: String,
        amend: bool,
        signoff: bool,
        push: bool,
        cancel: CancellationToken,
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
    /// Create PR (`gh pr create` or `glab mr create`) on an explicit click.
    CreatePr,
    Log,
    Compare {
        base: Option<String>,
    },
    /// Branches, remote branches, and tags for Compare with… (read-only).
    CompareTargets,
    /// The commit a typed revision names, for Compare with… (read-only).
    VerifyRevision(String),
    /// Commits that changed a file, for Compare with Revision… (read-only).
    FileHistory(PathBuf),
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
    /// Perforce Sync, Submit, Shelve or Unshelve: a cancellable job with a
    /// long cap, like `Commit`.
    PerforceRun(crate::perforce::Run),
    /// A branch, sync, or commit action (explicit click, quick cap).
    Branch(BranchOp),
}

impl Op {
    /// What this action can change in a commit that runs in its
    /// repository. A second commit has its own guard (`start_commit`).
    pub(crate) fn commit_risk(&self) -> CommitRisk {
        match self {
            Self::Stage(_) | Self::StageAll(_) | Self::Unstage(_) | Self::Discard { .. } => {
                CommitRisk::Index
            }
            // Branch and sync actions move `HEAD`, the index, or the refs
            // the commit's push reads: they wait until the push ends.
            Self::Switch(_) | Self::Branch(_) => CommitRisk::Head,
            Self::Shelve(_) | Self::Unshelve(_) | Self::DropShelf(_) => CommitRisk::Stash,
            Self::Perforce(_)
            | Self::PerforceRun(_)
            | Self::Delete(_)
            | Self::Commit { .. }
            | Self::Shelves
            | Self::Branches
            | Self::HeadMessage
            | Self::CreatePr
            | Self::Log
            | Self::Compare { .. }
            | Self::CompareTargets
            | Self::VerifyRevision(_)
            | Self::FileHistory(_)
            | Self::CreateFile(_)
            | Self::CreateDir(_)
            | Self::Duplicate(_)
            | Self::Rename { .. }
            | Self::Move { .. }
            | Self::Copy { .. }
            | Self::Reveal(_) => CommitRisk::None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Report {
    Message(String),
    /// The commit landed and its push runs next: progress, not the end.
    Pushing,
    Branches(Vec<String>),
    HeadMessage(String),
    Shelves(Vec<(String, String)>),
    Log(Vec<(String, String)>),
    Compare(CompareData),
    CompareTargets(Vec<crate::revision_compare::CompareTarget>),
    /// The full hash of a checked revision.
    Revision(String),
    History(Vec<crate::git_log::HistoryEntry>),
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
        Op::Perforce(_) | Op::PerforceRun(_) => {
            bail!("Perforce actions require the async process service")
        }
        Op::Stage(path) => git_path(GitPathInput {
            root,
            prefix: &["add", "--"],
            path: &path,
            verb: "Staged",
        }),
        Op::StageAll(paths) => stage_paths(root, &paths),
        Op::Unstage(path) => git_path(GitPathInput {
            root,
            prefix: &["restore", "--staged", "--"],
            path: &path,
            verb: "Unstaged",
        }),
        Op::Discard { path, untracked } => discard(root, &path, untracked),
        Op::Delete(path) => delete(root, &path),
        Op::Commit { .. } => bail!("Commit and push run as cancellable jobs"),
        Op::Shelve(paths) => shelve(root, &paths),
        Op::Shelves => Ok(Report::Shelves(shelves(root)?)),
        Op::Unshelve(reference) => unshelve(root, &reference),
        Op::DropShelf(reference) => drop_shelf(root, &reference),
        Op::Branches => Ok(Report::Branches(branches(root)?)),
        Op::HeadMessage => head_message(root),
        Op::Switch(name) => switch(root, &name),
        Op::CreatePr => create_pr(root),
        Op::Log => Ok(Report::Log(log(root)?)),
        Op::Compare { base } => Ok(Report::Compare(compare(root, base.as_deref())?)),
        Op::CompareTargets => crate::revision_compare::list_targets(root)
            .map(Report::CompareTargets)
            .map_err(anyhow::Error::msg),
        Op::VerifyRevision(text) => crate::revision_compare::verify_revision(root, &text)
            .map(Report::Revision)
            .map_err(anyhow::Error::msg),
        Op::FileHistory(path) => crate::git_log::file_history(root, &path)
            .map(Report::History)
            .map_err(anyhow::Error::msg),
        Op::CreateFile(path) => create_file(root, &path),
        Op::CreateDir(path) => create_dir(root, &path),
        Op::Duplicate(path) => duplicate(root, &path),
        Op::Move { from, to_dir } => move_entries(root, &from, &to_dir),
        Op::Copy { from, to_dir } => copy_entries(root, &from, &to_dir),
        Op::Rename { from, to } => rename(root, &from, &to),
        Op::Reveal(path) => reveal_entry(root, &path),
        Op::Branch(op) => branch::perform(root, &op),
    }
}

#[derive(Clone, Debug)]
pub enum NamePrompt {
    File {
        dir: PathBuf,
        name: String,
    },
    Folder {
        dir: PathBuf,
        name: String,
    },
    /// `dir` is read once when the prompt opens, never per frame.
    Rename {
        from: PathBuf,
        name: String,
        dir: bool,
    },
}

pub fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
