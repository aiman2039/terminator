//! The status-bar error. Its kind is kept next to the text, so behavior
//! never depends on the wording, and every write goes through one type
//! that also owns the dismiss state.
use std::collections::HashSet;
use std::path::PathBuf;

use crate::{daemon_connection, installation, retry_budget};

/// What a status error means to the rest of the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// The session service cannot be reached. A reconnect clears it.
    Connection,
    /// The service lost its terminal helper. A healthy helper clears it.
    Helper,
    /// A service authentication file is unreadable. A dismissal holds for
    /// that text even after the working directory changes.
    SessionAuth,
    /// A path disappeared. Repeats of a new path are limited.
    MissingPath,
    Other,
}

impl StatusKind {
    /// Kind of an untyped worker error (`Update::Error`). This is the only
    /// place text is read for its meaning; the kind is stored with it.
    pub(crate) fn of_report(text: &str) -> Self {
        if daemon_connection::is_connection_error(text) {
            Self::Connection
        } else if installation::is_helper_error(text) {
            Self::Helper
        } else if text.starts_with("Cannot read session service authentication file ") {
            Self::SessionAuth
        } else if text
            .lines()
            .any(|line| line.trim() == "No such file or directory (os error 2)")
        {
            Self::MissingPath
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusError {
    pub(crate) kind: StatusKind,
    pub(crate) text: String,
}

/// The shown error and its dismiss state. The shown text is never the
/// dismissed text.
#[derive(Debug, Default)]
pub(crate) struct StatusLine {
    current: Option<StatusError>,
    /// Same status text the user dismissed. Identical reports stay hidden.
    dismissed: Option<String>,
    /// Authentication errors belong to a service, not the selected directory.
    dismissed_auth: HashSet<String>,
    /// Working directory the dismiss latch and missing-path budget belong to.
    cwd: Option<PathBuf>,
    missing_path_reports: u8,
}

impl StatusLine {
    pub(crate) fn new(text: Option<String>) -> Self {
        let mut line = Self::default();
        if let Some(text) = text {
            line.set(text);
        }
        line
    }

    pub(crate) fn get(&self) -> Option<&StatusError> {
        self.current.as_ref()
    }

    pub(crate) fn text(&self) -> Option<&str> {
        self.current.as_ref().map(|error| error.text.as_str())
    }

    pub(crate) fn is(&self, kind: StatusKind) -> bool {
        self.current
            .as_ref()
            .is_some_and(|error| error.kind == kind)
    }

    /// The direct result of a user action: always shown. Showing the
    /// dismissed text again ends that dismissal.
    pub(crate) fn set(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.dismissed.as_deref() == Some(text.as_str()) {
            self.dismissed = None;
        }
        self.current = Some(StatusError {
            kind: StatusKind::Other,
            text,
        });
    }

    /// A background report that may repeat: a dismissed text stays
    /// hidden, and a new missing path shows only a few times.
    pub(crate) fn report(&mut self, cwd: Option<PathBuf>, text: String) {
        let kind = StatusKind::of_report(&text);
        if kind == StatusKind::SessionAuth && self.dismissed_auth.contains(&text) {
            return;
        }
        if self.cwd != cwd {
            self.cwd = cwd;
            self.dismissed = None;
            self.missing_path_reports = 0;
        }
        if self.dismissed.as_deref() == Some(text.as_str()) {
            return;
        }
        if self.text() != Some(text.as_str()) {
            if kind == StatusKind::MissingPath {
                if self.missing_path_reports >= retry_budget::MISSING_PATH_RETRY_LIMIT {
                    return;
                }
                self.missing_path_reports = self.missing_path_reports.saturating_add(1);
            } else {
                self.dismissed = None;
                self.missing_path_reports = 0;
            }
        }
        self.current = Some(StatusError { kind, text });
    }

    pub(crate) fn dismiss(&mut self) {
        let Some(error) = self.current.take() else {
            return;
        };
        if error.kind == StatusKind::SessionAuth {
            self.dismissed_auth.insert(error.text.clone());
        }
        self.dismissed = Some(error.text);
    }

    pub(crate) fn clear(&mut self) {
        self.current = None;
    }

    /// Clear the shown error only when it has this kind.
    pub(crate) fn clear_kind(&mut self, kind: StatusKind) {
        if self.is(kind) {
            self.current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissed_text_is_never_the_shown_text() {
        let mut line = StatusLine::default();
        line.report(None, "Settings rejected".into());
        line.dismiss();
        line.report(None, "Settings rejected".into());
        assert_eq!(line.text(), None);
        // The user runs the action again: its result shows, and a later
        // background repeat is no longer hidden behind the old dismissal.
        line.set("Settings rejected");
        line.report(None, "Settings rejected".into());
        assert_eq!(line.text(), Some("Settings rejected"));
        assert_ne!(line.dismissed.as_deref(), line.text());
    }

    #[test]
    fn kind_is_kept_when_the_text_is_reworded_elsewhere() {
        let mut line = StatusLine::default();
        line.report(None, "Reconnecting: Session daemon unavailable".into());
        assert!(line.is(StatusKind::Connection));
        // A user-action error that quotes a transport error is not one.
        line.set("Could not start session service: Reconnecting: refused");
        line.clear_kind(StatusKind::Connection);
        assert!(line.text().is_some());
    }
}
