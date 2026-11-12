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

/// Where the shown error came from: the operation's name for the summary,
/// and the job that a Retry click queues again.
#[derive(Clone, Debug)]
pub(crate) struct ErrorOrigin {
    pub(crate) operation: &'static str,
    pub(crate) retry: Option<crate::workspace_ops::Op>,
}

/// One message in the status log.
#[derive(Clone, Debug)]
pub(crate) struct LogEntry {
    pub(crate) error: bool,
    pub(crate) text: String,
    pub(crate) at: std::time::Instant,
}

/// The status log keeps this many messages.
pub(crate) const LOG_LIMIT: usize = 20;

/// The shown error and its dismiss state. The shown text is never the
/// dismissed text.
#[derive(Debug, Default)]
pub(crate) struct StatusLine {
    current: Option<StatusError>,
    /// Origin of `current`, when the failed job told it.
    origin: Option<ErrorOrigin>,
    /// The last [`LOG_LIMIT`] errors and notes, oldest first.
    log: std::collections::VecDeque<LogEntry>,
    /// Errors logged since the log was last opened.
    unread: usize,
    /// The details popover of the shown error is open.
    pub(crate) details_open: bool,
    /// The message log popup is open.
    pub(crate) log_open: bool,
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
        self.show(StatusError {
            kind: StatusKind::Other,
            text,
        });
    }

    /// The origin of the shown error, if its job told it.
    pub(crate) fn origin(&self) -> Option<&ErrorOrigin> {
        self.current.as_ref().and(self.origin.as_ref())
    }

    /// One line for the status bar: `<operation> failed: <first line>`,
    /// without the `Command failed (exit status: N): ` prefix.
    pub(crate) fn summary(&self) -> Option<String> {
        let text = self.text()?;
        let first = first_line(text);
        Some(match self.origin() {
            Some(origin) => format!("{} failed: {first}", origin.operation),
            None => first.to_owned(),
        })
    }

    /// Show `error` and log it when its text is new.
    fn show(&mut self, error: StatusError) {
        if self.text() != Some(error.text.as_str()) {
            self.origin = None;
            self.details_open = false;
            self.push_log(true, error.text.clone());
        }
        self.current = Some(error);
    }

    /// Tell the origin of the shown error, when it still is `text`.
    pub(crate) fn attach_origin(&mut self, text: &str, origin: ErrorOrigin) {
        if self.text() == Some(text) {
            self.origin = Some(origin);
        }
    }

    /// Log an error that shows in its own panel, not the status bar.
    pub(crate) fn log_error(&mut self, text: &str) {
        self.push_log(true, text.to_owned());
    }

    /// Log an info note the status bar showed.
    pub(crate) fn note_info(&mut self, text: &str) {
        self.push_log(false, text.to_owned());
    }

    fn push_log(&mut self, error: bool, text: String) {
        if self.log.back().is_some_and(|last| last.text == text) {
            return;
        }
        if error {
            self.unread = self.unread.saturating_add(1);
        }
        self.log.push_back(LogEntry {
            error,
            text,
            at: std::time::Instant::now(),
        });
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
    }

    /// The logged messages, newest first.
    pub(crate) fn log(&self) -> impl Iterator<Item = &LogEntry> {
        self.log.iter().rev()
    }

    pub(crate) fn unread(&self) -> usize {
        self.unread
    }

    /// The log was opened: its errors are read.
    pub(crate) fn mark_read(&mut self) {
        self.unread = 0;
    }

    pub(crate) fn clear_log(&mut self) {
        self.log.clear();
        self.unread = 0;
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
        self.show(StatusError { kind, text });
    }

    pub(crate) fn dismiss(&mut self) {
        self.details_open = false;
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
        self.details_open = false;
    }

    /// Clear the shown error only when it has this kind.
    pub(crate) fn clear_kind(&mut self, kind: StatusKind) {
        if self.is(kind) {
            self.current = None;
        }
    }
}

/// The first non-empty line of `text`, trimmed, without the
/// `Command failed (exit status: N): ` prefix of a process error.
pub(crate) fn first_line(text: &str) -> &str {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    match line.strip_prefix("Command failed (") {
        Some(rest) => rest
            .split_once("): ")
            .map_or(line, |(_, message)| message.trim()),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_the_operation_and_drops_the_exit_status() {
        let mut line = StatusLine::default();
        let text = "Command failed (exit status: 1): remote rejected (non-fast-forward)\nhint: pull first\n";
        line.report(None, text.into());
        line.attach_origin(
            text,
            ErrorOrigin {
                operation: "Git push",
                retry: None,
            },
        );
        assert_eq!(
            line.summary().as_deref(),
            Some("Git push failed: remote rejected (non-fast-forward)")
        );
        line.set("Settings rejected\n");
        assert_eq!(line.summary().as_deref(), Some("Settings rejected"));
        assert!(line.origin().is_none(), "a new error drops the old origin");
    }

    #[test]
    fn the_log_keeps_the_last_twenty_messages_and_counts_unread_errors() {
        let mut line = StatusLine::default();
        line.note_info("Created https://example.com/pull/42");
        for n in 0..25 {
            line.set(format!("error {n}"));
        }
        assert_eq!(line.log().count(), LOG_LIMIT);
        assert_eq!(line.log().next().map(|e| e.text.as_str()), Some("error 24"));
        assert_eq!(line.unread(), 25);
        line.mark_read();
        assert_eq!(line.unread(), 0);
    }

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
