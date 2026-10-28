//! Explicit file and Git actions from the sidebar. Diff review stays read-only.
//! Every path has to stay inside the repository or project root.
mod entries;
#[cfg(test)]
mod tests;
mod vcs;

use entries::*;
pub(crate) use vcs::PUSH_FAILED;
use vcs::*;

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
        Op::Discard { path, untracked } => discard(root, &path, untracked),
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
        Op::Unshelve(reference) => unshelve(root, &reference),
        Op::DropShelf(reference) => drop_shelf(root, &reference),
        Op::Branches => Ok(Report::Branches(branches(root)?)),
        Op::HeadMessage => head_message(root),
        Op::Switch(name) => switch(root, &name),
        Op::Log => Ok(Report::Log(log(root)?)),
        Op::Compare { base } => Ok(Report::Compare(compare(root, base.as_deref())?)),
        Op::CreateFile(path) => create_file(root, &path),
        Op::CreateDir(path) => create_dir(root, &path),
        Op::Duplicate(path) => duplicate(root, &path),
        Op::Move { from, to_dir } => move_entries(root, &from, &to_dir),
        Op::Copy { from, to_dir } => copy_entries(root, &from, &to_dir),
        Op::Rename { from, to } => rename(root, &from, &to),
        Op::Reveal(path) => reveal_entry(root, &path),
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
