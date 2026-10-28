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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Core(#[from] anyhow::Error),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error("{context}")]
    OutsideRepository {
        context: &'static str,
        #[source]
        source: std::path::StripPrefixError,
    },
}

impl Error {
    fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod diffstat;
pub mod ignore;
pub mod snapshot;
pub mod status;

pub use diffstat::{Stat, parse_numstat};
pub use ignore::{DISPLAY_LIMIT, Entry, compare_entries, entries, entries_known, natural_cmp};
pub use snapshot::{
    SNAPSHOT_LIMIT, blob, check_text, find_record, relative_path, snapshots, worktree_file,
};
pub use status::{Change, GitGroup, decorations, parse_porcelain, status_description};
pub use terminator_core::git::{RunInput, command, command_with, run, run_blocking};
