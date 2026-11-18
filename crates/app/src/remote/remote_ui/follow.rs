//! The remote Explorer follows the focused SSH terminal's folder.
//!
//! The remote shell reports its folder with OSC 7 (`file://host/path`); the
//! GUI PTY reader keeps the newest report per terminal. The report is
//! untrusted terminal text and a display hint only: it moves the Explorer
//! root and nothing else. It never feeds lifecycle, presence or any daemon
//! request other than the usual read-only listing. A report from another
//! host than the first one the terminal reported (a nested `ssh`), or a
//! folder outside the project root, leaves the Explorer where it is.
use super::View;
use crate::App;
use percent_encoding::percent_decode_str;
use terminator_core::remote::{RemoteProject, validate_relative};

/// The last report applied to a view, so the Explorer moves only when the
/// focused terminal or its folder changes; a click elsewhere in between is
/// kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Followed {
    pub session: String,
    pub report: String,
}

/// Explorer state for following a terminal.
#[derive(Debug, Default)]
pub(crate) struct Follow {
    /// The user pinned the Explorer root: reports are ignored.
    pub pinned: bool,
    pub last: Option<Followed>,
    /// Label of the terminal the Explorer follows, while it does.
    pub source: Option<String>,
    /// Why the newest report was not applied.
    pub note: Option<String>,
    /// The OSC 7 setup lines show below the Explorer header.
    pub help: bool,
    pub help_shell: crate::onboard::osc7::Shell,
}

/// Host and decoded absolute path of an OSC 7 payload. Only `file:` URLs
/// with an absolute, NUL-free UTF-8 path are accepted.
pub(crate) fn parse_report(payload: &str) -> Option<(String, String)> {
    let url = url::Url::parse(payload).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let path = percent_decode_str(url.path()).decode_utf8().ok()?;
    if !path.starts_with('/') || path.contains('\0') {
        return None;
    }
    Some((
        url.host_str().unwrap_or_default().to_owned(),
        path.into_owned(),
    ))
}

/// `path` relative to the project `root`, or `None` when it is outside.
pub(crate) fn relative_to_root(root: &str, path: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let path = path.trim_end_matches('/');
    let relative = if path == root {
        ""
    } else {
        path.strip_prefix(root)?.strip_prefix('/')?
    };
    validate_relative(relative).ok()?;
    Some(relative.to_owned())
}

impl App {
    /// Moves the remote Explorer to the focused SSH terminal's folder when
    /// that terminal reported a new one. Returns whether the listing moved.
    pub(super) fn follow_remote_terminal(&mut self, project: &RemoteProject) -> bool {
        let Some((session, label)) = self
            .context_session()
            .filter(|s| s.project_id == project.id && s.remote.is_some())
            .map(|s| (s.id.clone(), s.label.clone()))
        else {
            return false;
        };
        let Some(report) = self.backends.get(&session).and_then(|b| b.reported_cwd()) else {
            return false;
        };
        let first_host = parse_report(&report).map(|(host, _)| host);
        let host = self
            .remote_ui
            .follow_hosts
            .entry(session.clone())
            .or_insert_with(|| first_host.unwrap_or_default())
            .clone();
        let view = self.remote_ui.views.entry(project.id.clone()).or_default();
        apply_report(ApplyInput {
            view,
            root: &project.root,
            host: &host,
            followed: Followed { session, report },
            label,
        })
    }
}

pub(super) struct ApplyInput<'a> {
    pub view: &'a mut View,
    pub root: &'a str,
    /// The host the terminal named in its first report.
    pub host: &'a str,
    pub followed: Followed,
    pub label: String,
}

/// Applies one report to `view`. Returns whether the listing moved.
pub(super) fn apply_report(input: ApplyInput<'_>) -> bool {
    let ApplyInput {
        view,
        root,
        host,
        followed,
        label,
    } = input;
    if view.follow.pinned || view.follow.last.as_ref() == Some(&followed) {
        return false;
    }
    let target = parse_report(&followed.report);
    view.follow.last = Some(followed);
    let Some((report_host, path)) = target else {
        return false;
    };
    if report_host != host {
        view.follow.source = None;
        view.follow.note = Some("The terminal is on another host".into());
        return false;
    }
    let Some(relative) = relative_to_root(root, &path) else {
        view.follow.source = None;
        view.follow.note = Some("The terminal is outside the project folder".into());
        return false;
    };
    view.follow.note = None;
    view.follow.source = Some(label);
    if relative == view.path {
        return false;
    }
    view.path = relative;
    view.entries.clear();
    view.files_loading = false;
    view.pending_path = None;
    view.files_polled = None;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(view: &mut View, session: &str, report: &str) -> bool {
        apply_report(ApplyInput {
            view,
            root: "/home/me/api",
            host: "box",
            followed: Followed {
                session: session.into(),
                report: report.into(),
            },
            label: "zsh".into(),
        })
    }

    #[test]
    fn reports_decode_host_and_path() {
        assert_eq!(
            parse_report("file://box/home/me/a%20b"),
            Some(("box".into(), "/home/me/a b".into()))
        );
        assert_eq!(
            parse_report("file:///srv"),
            Some((String::new(), "/srv".into()))
        );
        assert_eq!(parse_report("https://box/home"), None);
        assert_eq!(parse_report("file://box/a%00b"), None);
        assert_eq!(parse_report("not a url"), None);
    }

    #[test]
    fn only_folders_under_the_project_root_map() {
        assert_eq!(
            relative_to_root("/home/me/api", "/home/me/api"),
            Some(String::new())
        );
        assert_eq!(
            relative_to_root("/home/me/api/", "/home/me/api/src/x/"),
            Some("src/x".into())
        );
        assert_eq!(relative_to_root("/home/me/api", "/home/me/apiary"), None);
        assert_eq!(relative_to_root("/home/me/api", "/etc"), None);
        assert_eq!(relative_to_root("/home/me/api", "/home/me/api/../x"), None);
        assert_eq!(relative_to_root("/", "/etc"), Some("etc".into()));
    }

    #[test]
    fn explorer_moves_only_when_the_report_changes() {
        let mut view = View::default();
        assert!(apply(&mut view, "s1", "file://box/home/me/api/src"));
        assert_eq!(view.path, "src");
        assert_eq!(view.follow.source.as_deref(), Some("zsh"));
        // A click elsewhere stays until the terminal reports a new folder.
        view.path = "docs".into();
        assert!(!apply(&mut view, "s1", "file://box/home/me/api/src"));
        assert_eq!(view.path, "docs");
        assert!(apply(&mut view, "s1", "file://box/home/me/api"));
        assert_eq!(view.path, "");
    }

    #[test]
    fn pinned_other_host_and_outside_reports_leave_the_explorer() {
        let mut view = View::default();
        view.follow.pinned = true;
        assert!(!apply(&mut view, "s1", "file://box/home/me/api/src"));
        assert_eq!(view.path, "");
        view.follow.pinned = false;
        assert!(!apply(&mut view, "s1", "file://other/home/me/api/src"));
        assert_eq!(view.path, "");
        assert!(view.follow.note.is_some());
        assert!(!apply(&mut view, "s1", "file://box/etc"));
        assert_eq!(view.path, "");
        assert!(view.follow.source.is_none());
    }
}
