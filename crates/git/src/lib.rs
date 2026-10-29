//! Read-only Git vocabulary: status, ignore rules, and diff snapshots.
//!
//! This crate owns `git -C`, `GIT_OPTIONAL_LOCKS=0`, porcelain parsing, and
//! the snapshot sequence behind both diff viewers. It never paints, never
//! touches sessions, and never mutates repositories: worktree mutation stays
//! in `terminator-core` and `terminator-hook`. The command runners live in
//! core (core's own metadata refresh uses them, which forbids the reverse
//! dependency) and are re-exported here; every runner takes the caller's
//! [`terminator_core::CommandOptions`] so timeouts and limits stay per caller.
#![forbid(unsafe_code)]

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Directory exceeds the {DISPLAY_LIMIT}-entry display limit")]
    DirectoryLimit { path: PathBuf },
    #[error("Diff review supports regular files, not symlinks or submodules")]
    NotRegularFile { path: PathBuf },
    #[error("{context}")]
    OutsideRepository {
        context: &'static str,
        #[source]
        source: std::path::StripPrefixError,
    },
    #[error("Review path must be inside the repository")]
    PathEscapes { path: PathBuf },
    #[error("Diff file exceeds 1 MiB")]
    TooLarge { bytes: u64 },
    #[error("Invalid Git diff record")]
    InvalidDiffRecord { header: String },
    #[error("Missing Git path")]
    MissingPath { header: String },
    #[error("Missing rename target")]
    MissingRenameTarget { header: String },
    #[error("Resolve this file's merge conflict before opening a two-way diff")]
    Conflicted { path: PathBuf },
    #[error("This file is no longer in conflict; refresh Git status")]
    NotConflicted { path: PathBuf },
    #[error("This file has no staged changes; refresh Git status")]
    NoStagedChanges { path: PathBuf },
    #[error("This file has no working-tree changes; refresh Git status")]
    NoWorkingChanges { path: PathBuf },
    #[error("File no longer exists; refresh Git status")]
    Deleted { path: PathBuf },
    #[error(transparent)]
    Process(#[from] terminator_core::ProcessError),
    #[error(transparent)]
    AsyncProcess(#[from] terminator_core::async_process::AsyncProcessError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod diffstat;
pub mod ignore;
pub mod merge;
pub mod snapshot;
pub mod status;

pub use diffstat::{Stat, parse_numstat};
pub use ignore::{DISPLAY_LIMIT, Entry, compare_entries, entries, entries_known, natural_cmp};
pub use merge::{ConflictStages, conflict_stages, parse_unmerged};
pub use snapshot::{
    SNAPSHOT_LIMIT, SnapshotsInput, blob, check_text, find_record, relative_path, snapshots,
    worktree_file,
};
pub use status::{Change, GitGroup, decorations, parse_porcelain, status_description};
pub use terminator_core::git::{RunInput, command, command_with, run, run_blocking};
