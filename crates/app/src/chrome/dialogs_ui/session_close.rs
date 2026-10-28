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
        self.popups
            .window(ctx, title)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if let Some(s) = session {
                    self.close_session_summary(ui, &sid, &s);
                }
                ui.label("Keep it running in the background, or terminate its processes.");
                ui.horizontal(|ui| self.close_session_buttons(ui, &sid));
            });
    }

    fn close_session_summary(&mut self, ui: &mut egui::Ui, sid: &str, s: &Session) {
        ui.label(format!("{} · {}", s.label, s.cwd.display()));
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

    fn close_session_buttons(&mut self, ui: &mut egui::Ui, sid: &str) {
        let keep = ui.button("Keep running");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "close-session-keep", keep.rect);
        if keep.clicked() {
            self.remove_tab(sid);
            self.close_session = None;
        }
        let terminate = ui.button("Terminate");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "close-session-terminate", terminate.rect);
        if terminate.clicked() {
            self.send(Request::Stop {
                session: sid.to_owned(),
            });
            self.remove_tab(sid);
            self.close_session = None;
        }
        if ui.button("Cancel").clicked() {
            self.close_session = None;
        }
    }

    pub(crate) fn poll_workspace_close(&mut self, ctx: &egui::Context) {
        loop {
            let Some((project, tab_id)) = self.close_workspace.clone() else {
                return;
            };
            // Native buffers die with the tab and bypass the dock close hook,
            // so guard them before the session checks below.
            let dirty = self.dirty_native_in_tab(&project, &tab_id);
            if let Some(path) = dirty.first() {
                self.native_close_prompt = Some((path.clone(), None));
                self.abort_workspace_close();
                return;
            }
            let live = self.live_workspace_sessions(&project, &tab_id);
            if live.is_empty() {
                self.close_workspace_tab_now(&project, &tab_id);
                continue;
            }
            if self.editors_only(&live) {
                self.close_workspace = None;
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

    fn live_workspace_sessions(&self, project: &str, tab_id: &str) -> Vec<String> {
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
        let mut open = true;
        let mut decided = false;
        self.popups
            .window(ctx, "Close tab?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "This tab contains {} running session(s).",
                    live.len()
                ));
                ui.weak("Keep their processes running in the background, or terminate them.");
                ui.horizontal(|ui| {
                    let response = ui.button("Keep running");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "workspace-keep-running", response.rect);
                    let background = response.clicked();
                    let terminate = ui.button("Terminate sessions").clicked();
                    if background || terminate {
                        if terminate {
                            for session in &live {
                                self.send(Request::Stop {
                                    session: session.clone(),
                                });
                            }
                        }
                        self.close_workspace_tab_now(project, tab_id);
                        decided = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.abort_workspace_close();
                        decided = true;
                    }
                });
            });
        if !open && !decided {
            self.abort_workspace_close();
        }
    }
}
