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

pub mod ignore;
pub mod snapshot;
pub mod status;

pub use ignore::{Entry, entries, entries_known};
pub use snapshot::{
    SNAPSHOT_LIMIT, blob, check_text, find_record, relative_path, snapshots, worktree_file,
};
pub use status::{Change, GitGroup, decorations, parse_porcelain, status_description};
pub use terminator_core::git::{command, command_with, run, run_blocking};
