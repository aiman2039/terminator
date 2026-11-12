//! Session and workspace-tab close confirmation.
use crate::*;

struct ConfirmWorkspaceCloseInput<'a> {
    ctx: &'a egui::Context,
    project: &'a str,
    tab_id: &'a str,
    live: Vec<String>,
}

impl App {
    pub(super) fn close_session_modal(&mut self, ctx: &egui::Context) {
        let Some(sid) = self.close_session.clone() else {
            return;
        };
        let session = self.state.sessions.iter().find(|s| s.id == sid).cloned();
        if session
            .as_ref()
            .is_none_or(|session| !session.lifecycle.live())
        {
            self.remove_tab(&sid);
            self.close_session = None;
        } else if session.as_ref().is_some_and(|session| {
            !daemon_connection::session_owner_available(&self.state, session)
        }) {
            self.remove_tab(&sid);
            self.close_session = None;
            self.info =
                Some("Closed the view. The unavailable session owner was not stopped.".into());
        } else if self.editors_only(std::slice::from_ref(&sid)) {
            self.close_session = None;
            if !self.skip_editor_close_request(std::slice::from_ref(&sid)) {
                self.close_editors(
                    editor_close::Target::Pane(sid.clone()),
                    vec![sid],
                    editor_close::Mode::Check,
                );
            }
        } else if self.check_idle_close(editor_close::Target::Pane(sid.clone()), vec![sid.clone()])
        {
            // Confirmation follows only if daemon verification cannot close safely.
        } else {
            self.close_session_window(ctx, sid, session);
        }
    }

    fn close_session_window(&mut self, ctx: &egui::Context, sid: String, session: Option<Session>) {
        let title = if session
            .as_ref()
            .is_some_and(|s| s.kind == SessionKind::Editor)
        {
            "Close editor?"
        } else {
            "Close session?"
        };
        let body = session
            .as_ref()
            .map(|s| format!("{} · {}", s.label, s.cwd.display()))
            .unwrap_or_default();
        let window = self.popups.window(ctx, title);
        let dialog = appearance::Confirm::new(
            body,
            "Keep running leaves it in the background. Terminate stops its processes.",
        )
        .destructive(appearance::ConfirmButton::new("Terminate").id("close-session-terminate"))
        .default_action(appearance::ConfirmButton::new("Keep running").id("close-session-keep"));
        let answer = appearance::confirm(ctx, window, dialog, |ui| {
            if let Some(s) = &session {
                self.close_session_editor_note(ui, &sid, s);
            }
        });
        match answer {
            Some(appearance::ConfirmAnswer::Default) => {
                self.remove_tab(&sid);
                self.close_session = None;
            }
            Some(appearance::ConfirmAnswer::Destructive) => {
                self.send(Request::Stop {
                    session: sid.clone(),
                });
                self.remove_tab(&sid);
                self.close_session = None;
            }
            Some(appearance::ConfirmAnswer::Cancel) => {
                self.close_session = None;
                // Cancel stops the rest of a bulk close too.
                self.session_selection.close_queue.clear();
            }
            None => {}
        }
    }

    fn close_session_editor_note(&mut self, ui: &mut egui::Ui, sid: &str, s: &Session) {
        if s.kind != SessionKind::Editor {
            return;
        }
        ui.colored_label(
            appearance::color(&self.theme.status_waiting),
            "Unsaved editor buffers remain alive when backgrounded.",
        );
        if ui.button("Save all editor buffers").clicked() {
            self.send(Request::EditorSave {
                session: sid.to_owned(),
            });
        }
    }

    pub(crate) fn poll_workspace_close(&mut self, ctx: &egui::Context) {
        loop {
            let Some((project, tab_id)) = self.close_workspace.clone() else {
                return;
            };
            // Native buffers die with the tab and bypass the dock close hook,
            // so guard them before the session checks below.
            if self.workspace_close_waits_on_native(&project, &tab_id) {
                return;
            }
            let live = self.live_workspace_sessions(&project, &tab_id);
            if live.is_empty() {
                self.close_workspace_tab_now(&project, &tab_id);
                continue;
            }
            if self.editors_only(&live) {
                // The close stays current while the editors are checked,
                // so the rest of a bulk close waits for it.
                if !self.skip_editor_close_request(&live) {
                    self.close_editors(
                        editor_close::Target::Workspace(project, tab_id),
                        live,
                        editor_close::Mode::Check,
                    );
                }
                return;
            }
            if self.check_idle_close(
                editor_close::Target::Workspace(project.clone(), tab_id.clone()),
                live.clone(),
            ) {
                return;
            }
            self.confirm_workspace_close(ConfirmWorkspaceCloseInput {
                ctx,
                project: &project,
                tab_id: &tab_id,
                live,
            });
            return;
        }
    }

    /// A dirty native buffer pauses the close on its own prompt. Save or
    /// Discard resumes the queue; Cancel or a failed save stops it and
    /// says why (`stop_native_close_owner`). Prompts of other flows do
    /// not hold this close. Returns true while the close must not proceed.
    fn workspace_close_waits_on_native(&mut self, project: &str, tab_id: &str) -> bool {
        let owner = native_editor::CloseOwner::Workspace(project.to_owned(), tab_id.to_owned());
        if self.native_close_waits_for(&owner) || !self.pending_native_close.is_empty() {
            return true;
        }
        let Some(path) = self.dirty_native_in_tab(project, tab_id).into_iter().next() else {
            return false;
        };
        self.prompt_workspace_close(project, tab_id, path);
        true
    }

    /// Ask about one dirty native buffer for this tab's close. The tab
    /// becomes the current close unless another one is running; then the
    /// answer finishes this tab on its own (`resume_native_close_owner`).
    pub(crate) fn prompt_workspace_close(&mut self, project: &str, tab_id: &str, path: PathBuf) {
        let target = (project.to_owned(), tab_id.to_owned());
        if self.close_workspace.as_ref().is_none_or(|t| *t == target) {
            // Show the tab, so its file is on screen while it is asked about.
            if let Some(workspace) = self.layouts.get_mut(project) {
                tab_id.clone_into(&mut workspace.active);
            }
            self.close_workspace = Some(target);
        }
        self.ask_native_close(native_editor::NativeCloseRequest::new(
            path,
            None,
            native_editor::CloseOwner::Workspace(project.to_owned(), tab_id.to_owned()),
        ));
    }

    pub(crate) fn live_workspace_sessions(&self, project: &str, tab_id: &str) -> Vec<String> {
        let sessions = self
            .layouts
            .get(project)
            .and_then(|workspace| workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .map(|tab| {
                tab.layout
                    .iter_all_tabs()
                    .filter_map(|(_, tab)| match tab {
                        Tab::Terminal(sid) => Some(sid.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sessions
            .into_iter()
            .filter(|id| {
                self.state.sessions.iter().any(|session| {
                    &session.id == id
                        && session.lifecycle.live()
                        && daemon_connection::session_owner_available(&self.state, session)
                })
            })
            .collect()
    }

    fn confirm_workspace_close(&mut self, input: ConfirmWorkspaceCloseInput<'_>) {
        let ConfirmWorkspaceCloseInput {
            ctx,
            project,
            tab_id,
            live,
        } = input;
        let sessions = appearance::count_noun(live.len(), "session");
        let running = if live.len() == 1 { "is" } else { "are" };
        let terminate = format!("Terminate {sessions}");
        let window = self.popups.window(ctx, "Close tab?");
        let dialog = appearance::Confirm::new(
            format!("{sessions} {running} running in this tab."),
            "Keep running leaves them in the background. Terminate stops their processes.",
        )
        .destructive(appearance::ConfirmButton::new(&terminate).id("workspace-terminate"))
        .default_action(
            appearance::ConfirmButton::new("Keep running").id("workspace-keep-running"),
        );
        match appearance::confirm(ctx, window, dialog, |_| {}) {
            Some(appearance::ConfirmAnswer::Destructive) => {
                for session in &live {
                    self.send(Request::Stop {
                        session: session.clone(),
                    });
                }
                self.close_workspace_tab_now(project, tab_id);
            }
            Some(appearance::ConfirmAnswer::Default) => {
                self.close_workspace_tab_now(project, tab_id)
            }
            Some(appearance::ConfirmAnswer::Cancel) => self.abort_workspace_close(),
            None => {}
        }
    }
}
