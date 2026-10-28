//! Errors for the crate-root helpers: data paths, atomic writes, and shell discovery.
use std::path::PathBuf;

/// Why the data or runtime directories, or the session token, are unusable.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("No user data directory")]
    NoDataDirectory,
    #[error("No home directory")]
    NoHomeDirectory,
    #[error("Refusing symlink data directory")]
    SymlinkDataDirectory { path: PathBuf },
    #[error("Runtime path too long for Unix socket; use a shorter data directory")]
    SocketPathTooLong { socket: PathBuf },
    #[error("Cannot read session service authentication file {}", path.display())]
    Token {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Why [`crate::atomic_write`] could not replace a file.
#[derive(Debug, thiserror::Error)]
pub enum AtomicWriteError {
    #[error("Missing parent directory")]
    MissingParent { path: PathBuf },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// No default shell executable exists on this system.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(unix, error("No zsh, bash, or sh executable found"))]
#[cfg_attr(not(unix), error("No pwsh, powershell, or cmd executable found"))]
pub struct NoShellError;
