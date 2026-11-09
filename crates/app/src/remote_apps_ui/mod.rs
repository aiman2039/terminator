//! Remote app windows (`remote-apps-v1`): each toplevel window of a remote
//! app on an SSH project's host is a tab of that project. The daemon owns
//! the session; this module long-polls it, opens and closes tabs as windows
//! come and go, and paints each window from its shared-memory frame.
//! See docs/REMOTE_APPS.md.

mod cursor;
mod keys;
mod pixels;
mod pointer;
mod view;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
pub(crate) use pixels::{Pixels, Readers, RemoteAppsFrame, decode};
use terminator_core::remote_apps::{
    CAPABILITY, ClientMessage, ErrorKind, RemoteAppsState, RemoteAppsStatus, UNKNOWN_SESSION,
    WindowId, WindowRole,
};
use terminator_core::{Request, id};

use crate::{After, App, Job, Tab};

/// Wait after a failed poll before the next one.
const ERROR_BACKOFF: Duration = Duration::from_secs(2);
/// Repaint interval while sessions are polled, so the ticks keep running.
const TICK: Duration = Duration::from_millis(250);
/// A second close of a tab within this time closes it without the app.
const FORCE_CLOSE: Duration = Duration::from_secs(5);

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
    cursors: HashMap<String, cursor::CursorView>,
    /// Windows whose tab the user closed while the app ignored the close:
    /// they get no tab again while they exist.
    dismissed: HashSet<(String, u64)>,
}

#[derive(Default)]
struct Session {
    id: Option<String>,
    /// Shared so painting a tab does not copy the state every frame.
    state: Option<std::sync::Arc<RemoteAppsState>>,
    since: u64,
    fetching: bool,
    wait_until: Option<Instant>,
    error: Option<String>,
    /// The remote clipboard serial already seen.
    clipboard_serial: u64,
    readers: Readers,
}

impl Session {
    /// Ended sessions stop polling until the user connects again.
    fn ended(&self) -> bool {
        self.state.as_ref().is_some_and(|s| !s.status.live())
    }

    /// Drops what belongs to the daemon session, so the next poll connects
    /// (or reads) from scratch. Serials restart in a new session.
    fn forget(&mut self) {
        self.id = None;
        self.state = None;
        self.since = 0;
        self.clipboard_serial = 0;
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
        ctx.memory(|m| m.focused())
            .is_some_and(|id| self.is_remote_pane(id))
    }

    /// F8: give the keys back to Terminator.
    pub(crate) fn release_remote_app_focus(&mut self, ctx: &egui::Context) {
        if let Some(id) = ctx.memory(|m| m.focused())
            && self.is_remote_pane(id)
        {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }

    fn is_remote_pane(&self, id: egui::Id) -> bool {
        self.remote_apps
            .windows
            .keys()
            .any(|(project, window)| view::pane_id(project, *window) == id)
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
        self.release_unpainted_remote_windows(ctx);
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
                readers: Arc::clone(&session.readers),
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
                // A restarted daemon forgot the session: connect again. The
                // tabs stay; the windows come back with the same IDs.
                if error.contains(UNKNOWN_SESSION) {
                    session.forget();
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
            session.forget();
        }
        session.id = Some(state.session.clone());
        session.error = None;
        let mut since = if torn { session.since } else { state.revision };
        for (window, pixels) in images {
            let applied = self
                .remote_apps
                .windows
                .entry((project.to_owned(), window))
                .or_default()
                .set_pixels(ctx, || format!("remote-app:{project}:{window}"), pixels);
            if !applied {
                // Patches for a texture we do not have: read everything.
                since = 0;
            }
        }
        if let Some(session) = self.remote_apps.sessions.get_mut(project) {
            session.since = since;
        }
        self.remote_apps
            .cursors
            .entry(project.to_owned())
            .or_default()
            .update(ctx, state.cursor.as_ref());
        self.take_remote_clipboard(ctx, project, &state);
        self.sync_remote_tabs(project, &state);
        if let Some(session) = self.remote_apps.sessions.get_mut(project) {
            session.state = Some(std::sync::Arc::new(state));
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
        if state.status == RemoteAppsStatus::Connected {
            self.remote_apps
                .dismissed
                .retain(|(p, w)| p != project || toplevels.iter().any(|(id, _)| id == w));
        }
        for &(window, title) in &toplevels {
            if self
                .remote_apps
                .dismissed
                .contains(&(project.to_owned(), window))
            {
                continue;
            }
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
    /// request, or the user asked again within [`FORCE_CLOSE`] (the app
    /// ignored it), so the caller closes the tab itself. The app keeps
    /// running on the host then.
    pub(crate) fn remote_app_close_requested(&mut self, project: &str, window: u64) -> bool {
        let Some(session) = self.remote_apps.sessions.get(project) else {
            return false;
        };
        let live = session
            .state
            .as_ref()
            .is_some_and(|s| s.status == RemoteAppsStatus::Connected);
        let key = (project.to_owned(), window);
        let now = Instant::now();
        let asked_again = self
            .remote_apps
            .windows
            .get(&key)
            .and_then(|v| v.close_asked)
            .is_some_and(|at| now.duration_since(at) < FORCE_CLOSE);
        if let (Some(id), true, false) = (&session.id, live, asked_again) {
            let id = id.clone();
            self.remote_apps.windows.entry(key).or_default().close_asked = Some(now);
            self.send_remote_input(
                id,
                vec![ClientMessage::CloseRequested {
                    id: WindowId(window),
                }],
            );
            return true;
        }
        if asked_again {
            // The window still exists on the host: do not open it again.
            self.remote_apps.dismissed.insert(key.clone());
        }
        self.remote_apps.windows.remove(&key);
        false
    }

    /// A focused window whose tab was not painted this pass (another tab
    /// came to the front) lets go of its keys and loses focus on the host,
    /// so the app sees no stuck key.
    fn release_unpainted_remote_windows(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        let mut sends: Vec<(String, Vec<ClientMessage>)> = Vec::new();
        for ((project, window), view) in &mut self.remote_apps.windows {
            let messages = view.release_if_unpainted(*window, pass);
            if messages.is_empty() {
                continue;
            }
            if let Some(session) = self
                .remote_apps
                .sessions
                .get(project)
                .and_then(|s| s.id.clone())
            {
                sends.push((session, messages));
            }
        }
        for (session, messages) in sends {
            self.send_remote_input(session, messages);
        }
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
