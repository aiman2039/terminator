//! Remote app windows (`remote-apps-v1`): each toplevel window of a remote
//! app on an SSH project's host is a tab of that project. The daemon owns
//! the session; this module long-polls it, opens and closes tabs as windows
//! come and go, and paints each window from its shared-memory frame.
//! See docs/REMOTE_APPS.md.

mod cursor;
mod keys;
mod view;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use eframe::egui::{self, ColorImage};
use terminator_core::remote_apps::{
    CAPABILITY, ClientMessage, RemoteAppsState, RemoteAppsStatus, WindowId, WindowRole,
};
use terminator_core::{Request, id};
use terminator_sys::{FrameRect, SharedFrameReader};

use crate::{After, App, Job, Tab};

/// Wait after a failed poll before the next one.
const ERROR_BACKOFF: Duration = Duration::from_secs(2);
/// Repaint interval while sessions are polled, so the ticks keep running.
const TICK: Duration = Duration::from_millis(250);

/// A state reply with the pixels of the windows that changed.
pub(crate) struct RemoteAppsFrame {
    pub(crate) state: RemoteAppsState,
    pub(crate) images: Vec<(u64, ColorImage)>,
    /// A frame changed while it was copied; poll again from the old revision.
    pub(crate) torn: bool,
}

/// Reads the pixels of windows changed after `since` (off the GUI thread).
pub(crate) fn decode(
    state: RemoteAppsState,
    since: u64,
    runtime: &Path,
) -> Result<RemoteAppsFrame> {
    let mut images = Vec::new();
    let mut torn = false;
    for window in state.windows.iter().filter(|w| w.revision > since) {
        let Some(path) = &window.shm else { continue };
        match read_window(
            path,
            (window.width, window.height),
            window.sequence,
            runtime,
        ) {
            Ok(Some(image)) => images.push((window.id.0, image)),
            // Torn copy, or the daemon moved the window to a new file.
            Ok(None) | Err(_) => torn = true,
        }
    }
    Ok(RemoteAppsFrame {
        state,
        images,
        torn,
    })
}

fn read_window(
    path: &Path,
    size: (u16, u16),
    sequence: u64,
    runtime: &Path,
) -> Result<Option<ColorImage>> {
    ensure!(
        path.starts_with(runtime) && !path.components().any(|c| c == Component::ParentDir),
        "Remote app frame is outside the runtime directory"
    );
    let reader = SharedFrameReader::open(path)?;
    ensure!(
        reader.size() == size,
        "Remote app frame has a different size"
    );
    let full = FrameRect {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
    };
    let Some(copies) = reader.read(sequence, &[full]) else {
        return Ok(None);
    };
    let (_, rgba) = copies.first().context("Empty remote app frame")?;
    Ok(Some(ColorImage::from_rgba_premultiplied(
        [usize::from(size.0), usize::from(size.1)],
        rgba,
    )))
}

pub(crate) fn tab_title(saved: &str) -> String {
    if saved.trim().is_empty() {
        "Remote app".into()
    } else {
        saved.to_owned()
    }
}

#[derive(Default)]
pub(crate) struct RemoteAppsGui {
    sessions: HashMap<String, Session>,
    windows: HashMap<(String, u64), view::WindowView>,
    /// egui IDs of the remote window panes painted this frame.
    painted: HashSet<egui::Id>,
    cursors: HashMap<String, cursor::CursorView>,
}

#[derive(Default)]
struct Session {
    id: Option<String>,
    state: Option<RemoteAppsState>,
    since: u64,
    fetching: bool,
    wait_until: Option<Instant>,
    error: Option<String>,
    /// The remote clipboard serial already seen.
    clipboard_serial: u64,
}

impl Session {
    /// Ended sessions stop polling until the user connects again.
    fn ended(&self) -> bool {
        self.state.as_ref().is_some_and(|s| !s.status.live())
    }
}

impl App {
    pub(crate) fn remote_apps_supported(&self) -> bool {
        self.state_loaded && self.state.capabilities.iter().any(|c| c == CAPABILITY)
    }

    /// Connects (or reattaches) a project's remote apps; its windows open as
    /// tabs.
    pub(crate) fn connect_remote_apps(&mut self, project: &str) {
        let session = self
            .remote_apps
            .sessions
            .entry(project.to_owned())
            .or_default();
        if session.ended() {
            *session = Session::default();
        }
        session.wait_until = None;
    }

    /// Whether a painted remote window has keyboard focus: then it gets
    /// every key and no Terminator shortcut runs.
    pub(crate) fn remote_app_captures_keys(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| {
            m.focused()
                .is_some_and(|id| self.remote_apps.painted.contains(&id))
        })
    }

    /// F8: give the keys back to Terminator.
    pub(crate) fn release_remote_app_focus(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused()
                && self.remote_apps.painted.contains(&id)
            {
                m.surrender_focus(id);
            }
        });
    }

    pub(crate) fn remote_apps_connected(&self, project: &str) -> bool {
        self.remote_apps
            .sessions
            .get(project)
            .is_some_and(|s| s.id.is_some() && !s.ended())
    }

    pub(crate) fn disconnect_remote_apps(&mut self, project: &str) {
        if let Some(session) = self.remote_apps.sessions.get(project)
            && let Some(id) = &session.id
        {
            self.send(Request::RemoteAppsDisconnect {
                session: id.clone(),
            });
        }
    }

    /// Per frame: reconnects projects whose layout has remote windows, and
    /// keeps one state poll in flight for every live session.
    pub(crate) fn remote_apps_tick(&mut self, ctx: &egui::Context) {
        if !self.remote_apps_supported() {
            return;
        }
        let restored: Vec<String> = self
            .layouts
            .iter()
            .filter(|(project, workspace)| {
                !self.remote_apps.sessions.contains_key(*project)
                    && workspace
                        .tabs
                        .iter()
                        .flat_map(|tab| tab.layout.iter_all_tabs())
                        .any(|(_, pane)| matches!(pane, Tab::RemoteApp { .. }))
            })
            .map(|(project, _)| project.clone())
            .collect();
        for project in restored {
            self.connect_remote_apps(&project);
        }
        let now = Instant::now();
        let mut polling = false;
        for (project, session) in &mut self.remote_apps.sessions {
            if session.ended() {
                continue;
            }
            polling = true;
            if session.fetching || session.wait_until.is_some_and(|t| now < t) {
                continue;
            }
            let job = Job::RemoteApps {
                project: project.clone(),
                session: session.id.clone(),
                since: session.since,
            };
            session.fetching = self.jobs.send(job).is_ok();
        }
        if polling {
            ctx.request_repaint_after(TICK);
        }
    }

    pub(crate) fn handle_remote_apps(
        &mut self,
        ctx: &egui::Context,
        project: &str,
        result: Result<Option<Box<RemoteAppsFrame>>, String>,
    ) {
        let Some(session) = self.remote_apps.sessions.get_mut(project) else {
            return;
        };
        session.fetching = false;
        let frame = match result {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(error) => {
                // A restarted daemon forgot the session: connect again.
                if error.contains("Unknown remote-apps session") {
                    session.id = None;
                    session.since = 0;
                }
                session.error = Some(error);
                session.wait_until = Instant::now().checked_add(ERROR_BACKOFF);
                return;
            }
        };
        let RemoteAppsFrame {
            state,
            images,
            torn,
        } = *frame;
        if session.id.as_ref() != Some(&state.session) {
            session.since = 0;
        }
        session.id = Some(state.session.clone());
        session.error = None;
        if !torn {
            session.since = state.revision;
        }
        for (window, image) in images {
            self.remote_apps
                .windows
                .entry((project.to_owned(), window))
                .or_default()
                .set_image(ctx, format!("remote-app:{project}:{window}"), image);
        }
        self.remote_apps
            .cursors
            .entry(project.to_owned())
            .or_default()
            .update(ctx, state.cursor.as_ref());
        self.take_remote_clipboard(ctx, project, &state);
        self.sync_remote_tabs(project, &state);
        if let Some(session) = self.remote_apps.sessions.get_mut(project) {
            session.state = Some(state);
        }
        ctx.request_repaint();
    }

    /// Puts text a remote app copied on the local clipboard, but only while
    /// one of this host's windows has focus: a background app must not
    /// replace the clipboard. The first state after connecting only sets
    /// the baseline.
    fn take_remote_clipboard(
        &mut self,
        ctx: &egui::Context,
        project: &str,
        state: &RemoteAppsState,
    ) {
        let Some(clipboard) = &state.clipboard else {
            return;
        };
        let focused = self
            .remote_apps
            .windows
            .iter()
            .any(|((p, _), view)| p == project && view.has_focus());
        let Some(session) = self.remote_apps.sessions.get_mut(project) else {
            return;
        };
        let new = clipboard.serial > session.clipboard_serial;
        let first = session.clipboard_serial == 0 && session.state.is_none();
        session.clipboard_serial = clipboard.serial;
        if new && focused && !first {
            ctx.copy_text(clipboard.text.clone());
        }
    }

    /// Opens a tab for each new toplevel window, renames tabs, and, while
    /// connected, closes tabs whose window is gone. While reconnecting the
    /// tabs stay: the windows come back with the same IDs.
    fn sync_remote_tabs(&mut self, project: &str, state: &RemoteAppsState) {
        let toplevels: Vec<(u64, &str)> = state
            .windows
            .iter()
            .filter(|w| matches!(w.role, WindowRole::Toplevel))
            .map(|w| (w.id.0, w.title.as_str()))
            .collect();
        for &(window, title) in &toplevels {
            let tab = Tab::RemoteApp {
                project: project.to_owned(),
                window,
                title: title.to_owned(),
            };
            let present = self.layouts.get(project).is_some_and(|w| w.contains(&tab));
            if present {
                if let Some(workspace) = self.layouts.get_mut(project) {
                    workspace.retitle_remote_app(&tab.key(), title);
                }
            } else {
                self.place_gui_tab(project.to_owned(), tab, After::Workspace(id(), Vec::new()));
            }
        }
        if state.status != RemoteAppsStatus::Connected {
            return;
        }
        let open: HashSet<u64> = toplevels.iter().map(|(w, _)| *w).collect();
        let gone: Vec<String> = self
            .layouts
            .get(project)
            .into_iter()
            .flat_map(|w| w.tabs.iter())
            .flat_map(|tab| tab.layout.iter_all_tabs())
            .filter_map(|(_, pane)| match pane {
                Tab::RemoteApp { window, .. } if !open.contains(window) => Some(pane.key()),
                _ => None,
            })
            .collect();
        if let Some(workspace) = self.layouts.get_mut(project) {
            for key in &gone {
                workspace.remove_key(key);
            }
        }
        self.remote_apps
            .windows
            .retain(|(p, w), _| p != project || state.windows.iter().any(|s| s.id.0 == *w));
    }

    /// Asks the app to close `window`. `false` when no session can carry the
    /// request, so the caller closes the tab itself.
    pub(crate) fn remote_app_close_requested(&mut self, project: &str, window: u64) -> bool {
        let Some(session) = self.remote_apps.sessions.get(project) else {
            return false;
        };
        let live = session
            .state
            .as_ref()
            .is_some_and(|s| s.status == RemoteAppsStatus::Connected);
        if let (Some(id), true) = (&session.id, live) {
            self.send_remote_input(
                id.clone(),
                vec![ClientMessage::CloseRequested {
                    id: WindowId(window),
                }],
            );
            return true;
        }
        self.remote_apps
            .windows
            .remove(&(project.to_owned(), window));
        false
    }

    fn send_remote_input(&self, session: String, messages: Vec<ClientMessage>) {
        for chunk in messages.chunks(terminator_core::remote_apps::MAX_INPUT) {
            self.send(Request::RemoteAppsInput {
                session: session.clone(),
                messages: chunk.to_vec(),
            });
        }
    }
}
