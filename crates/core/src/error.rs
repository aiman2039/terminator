//! Errors for the crate-root helpers: data paths, atomic writes, and shell
//! discovery, plus the formatters that print an error with its sources.
use std::{error::Error, fmt, path::PathBuf};

/// Displays an error followed by each of its sources, separated by `": "`.
/// This is the text `{:#}` printed when these errors were `anyhow` errors.
pub struct ErrorChain<'a>(pub &'a (dyn Error + 'static));
impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(error) = source {
            write!(f, ": {error}")?;
            source = error.source();
        }
        Ok(())
    }
}

/// Debug-formats an error the way `anyhow` did: the message, then a
/// `Caused by:` list of its sources.
pub struct ErrorReport<'a>(pub &'a (dyn Error + 'static));
impl fmt::Debug for ErrorReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let Some(cause) = self.0.source() else {
            return Ok(());
        };
        write!(f, "\n\nCaused by:")?;
        let multiple = cause.source().is_some();
        let mut next = Some(cause);
        let mut number: usize = 0;
        while let Some(error) = next {
            writeln!(f)?;
            let text = error.to_string();
            for (line_index, line) in text.split('\n').enumerate() {
                match (line_index, multiple) {
                    (0, true) => write!(f, "{number: >5}: ")?,
                    (0, false) => f.write_str("    ")?,
                    (_, true) => f.write_str("\n       ")?,
                    (_, false) => f.write_str("\n    ")?,
                }
                f.write_str(line)?;
            }
            number = number.saturating_add(1);
            next = error.source();
        }
        Ok(())
    }
}

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
