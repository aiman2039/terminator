use super::super::super::*;
use super::super::tabs::{TabFace, TabLeading};
use crate::workspace::WorkspaceTab;

impl TabFace {
    /// A face with no session, brand, or status: just a label and icon.
    fn plain(label: String, icon: &'static str) -> Self {
        Self {
            label,
            icon,
            image: None,
            sid: None,
            brand: None,
            status: None,
        }
    }

    fn file_name(path: &std::path::Path) -> String {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

impl App {
    /// The tab a top-level group shows on the strip: its remembered
    /// primary while still in the layout, else its first pane.
    pub(super) fn group_primary(group: &WorkspaceTab) -> Option<&Tab> {
        group
            .primary
            .as_ref()
            .filter(|tab| group.layout.find_tab(tab).is_some())
            .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab))
    }

    /// Workspace strip tab face: the focused terminal's label, kind icon,
    /// stable agent brand, and hook lifecycle status, all from the shared
    /// agent presentation model.
    pub(crate) fn tab_face(&self, primary: Option<&Tab>) -> TabFace {
        match primary {
            Some(Tab::Terminal(sid)) => self.terminal_tab_face(sid),
            Some(Tab::Diff { path, .. }) => TabFace::plain(TabFace::file_name(path), "FileDiff"),
            Some(Tab::Image { path }) => TabFace::plain(TabFace::file_name(path), "FileImage"),
            Some(Tab::Browser { id, target }) => TabFace {
                image: self.browser_favicon(id),
                ..TabFace::plain(self.browser_title(id, target), "Globe")
            },
            Some(Tab::NativeEditor { path }) => {
                TabFace::plain(TabFace::file_name(path), "FileCode")
            }
            Some(Tab::Player) => TabFace::plain("Player".into(), "FileMusic"),
            Some(Tab::CommitLog { .. }) => TabFace::plain("Commit Log".into(), "FileDiff"),
            Some(Tab::Blame { path, .. }) => TabFace::plain(
                format!(
                    "Blame {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                "FileDiff",
            ),
            Some(tab @ (Tab::RevisionDiff { .. } | Tab::FileHistory { .. })) => TabFace::plain(
                tab.git_title(),
                if matches!(tab, Tab::FileHistory { .. }) {
                    "History"
                } else {
                    "FileDiff"
                },
            ),
            Some(tab @ Tab::Merge { .. }) => TabFace::plain(tab.git_title(), "GitCompareArrows"),
            Some(Tab::Vnc {
                label,
                session,
                host,
                port,
                ..
            }) => TabFace {
                label: Self::vnc_tab_label(label, host, *port),
                icon: "Monitor",
                image: None,
                sid: None,
                brand: None,
                status: self.vnc_tab_status(session),
            },
            None => TabFace::plain("Workspace".into(), "Terminal"),
        }
    }

    fn terminal_tab_face(&self, sid: &String) -> TabFace {
        self.state
            .sessions
            .iter()
            .find(|s| &s.id == sid)
            .map(|s| {
                let presented = self.present_session(sid);
                TabFace {
                    label: s.label.clone(),
                    icon: self.terminal_title_icon(
                        sid,
                        if s.kind == SessionKind::Editor {
                            "FileCode"
                        } else {
                            "Terminal"
                        },
                    ),
                    image: None,
                    sid: Some(sid.clone()),
                    brand: presented.brand_icon,
                    status: presented.lifecycle.map(|lifecycle| {
                        (
                            lifecycle,
                            presented.status_icon,
                            crate::sidebar_ui::state_color(lifecycle, &self.theme),
                        )
                    }),
                }
            })
            .unwrap_or(TabFace::plain("Terminal".into(), "Terminal"))
    }

    fn vnc_tab_label(label: &str, host: &str, port: u16) -> String {
        if !label.trim().is_empty() {
            label.to_owned()
        } else if !host.trim().is_empty() {
            format!("{host}:{port}")
        } else {
            "VNC Viewer".into()
        }
    }

    /// Connection state replaces the kind glyph, the way hook lifecycle
    /// does for terminals: one glance answers "is the remote still up"
    /// without reading the pane.
    fn vnc_tab_status(&self, session: &str) -> Option<(AgentState, &'static str, Color32)> {
        self.state
            .vnc_sessions
            .iter()
            .find(|candidate| candidate.id == session)
            .map(|candidate| {
                use terminator_core::vnc::VncSessionStatus;
                let (state, icon) = match candidate.status {
                    VncSessionStatus::Connected => (AgentState::Running, "Circle"),
                    VncSessionStatus::Connecting => (AgentState::Running, "LoaderCircle"),
                    VncSessionStatus::Failed => (AgentState::Failed, "CircleAlert"),
                    VncSessionStatus::Disconnected => (AgentState::Stopped, "CircleStop"),
                };
                (
                    state,
                    icon,
                    crate::sidebar_ui::state_color(state, &self.theme),
                )
            })
    }

    /// Resolved leading icons for a terminal tab. Process inspection supplies
    /// the brand before a hook exists. Hook lifecycle then replaces the
    /// session-kind glyph (spinning while running). Unknown sessions resolve
    /// to no icons.
    pub(in super::super) fn tab_leading(&self, sid: &str) -> TabLeading {
        let presented = self.present_session(sid);
        let kind = self.state.sessions.iter().find(|s| s.id == sid).map(|s| {
            if s.kind == SessionKind::Editor {
                "FileCode"
            } else {
                "Terminal"
            }
        });
        let status = presented.lifecycle.map(|lifecycle| {
            (
                presented.status_icon,
                crate::sidebar_ui::state_color(lifecycle, &self.theme),
                lifecycle == AgentState::Running,
            )
        });
        TabLeading {
            brand: presented.brand_icon,
            kind: status
                .is_none()
                .then_some(kind.map(|icon| self.terminal_title_icon(sid, icon)))
                .flatten(),
            status,
        }
    }
}
