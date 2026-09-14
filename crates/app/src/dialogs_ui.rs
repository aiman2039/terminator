//! Modal rendering.
use super::{AttentionAction, AttentionCard, attention_card, *};

impl App {
    pub(super) fn modals(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.add_project && !self.picker_active {
            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                eprintln!("Fixture native project picker opened");
            }
            self.add_project = false;
            self.picker_active = true;
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(frame)
                .set_title("Open project");
            let cwd = self.dialog_directory();
            self.selection_generation = self.selection_generation.wrapping_add(1);
            let generation = self.selection_generation;
            let tx = self.update_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let dialog = dialog.set_directory(services::existing_directory(cwd));
                let path = pollster::block_on(dialog.pick_folder()).map(|p| p.path().to_path_buf());
                let _ = tx.send(Update::PickedProject(path, generation));
                ctx.request_repaint();
            });
        }
        if let Some((project, tab_id)) = self.close_workspace.clone() {
            let sessions = self
                .layouts
                .get(&project)
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
            let live: Vec<_> = sessions
                .iter()
                .filter(|id| {
                    self.state
                        .sessions
                        .iter()
                        .any(|s| &s.id == *id && s.lifecycle.live())
                })
                .cloned()
                .collect();
            if live.is_empty() {
                if let Some(workspace) = self.layouts.get_mut(&project) {
                    workspace.close(&tab_id);
                }
                self.close_workspace = None;
            } else if self.editors_only(&live) {
                self.close_workspace = None;
                if !self.skip_editor_close_request(&live) {
                    self.close_editors(
                        editor_close::Target::Workspace(project, tab_id),
                        live,
                        editor_close::Mode::Check,
                    );
                }
            } else {
                let mut open = true;
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
                        ui.weak(
                            "Keep their processes running in the background, or terminate them.",
                        );
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
                                if let Some(workspace) = self.layouts.get_mut(&project) {
                                    workspace.close(&tab_id);
                                }
                                self.close_workspace = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.close_workspace = None;
                            }
                        });
                    });
                if !open {
                    self.close_workspace = None;
                }
            }
        }
        if let Some(sid) = self.close_session.clone() {
            let session = self.state.sessions.iter().find(|s| s.id == sid).cloned();
            if session
                .as_ref()
                .is_none_or(|session| !session.lifecycle.live())
            {
                self.remove_tab(&sid);
                self.close_session = None;
            } else if self.editors_only(std::slice::from_ref(&sid)) {
                self.close_session = None;
                if !self.skip_editor_close_request(std::slice::from_ref(&sid)) {
                    self.close_editors(
                        editor_close::Target::Pane(sid.clone()),
                        vec![sid],
                        editor_close::Mode::Check,
                    );
                }
            } else {
                self.popups
                    .window(
                        ctx,
                        if session
                            .as_ref()
                            .is_some_and(|s| s.kind == SessionKind::Editor)
                        {
                            "Close editor?"
                        } else {
                            "Close session?"
                        },
                    )
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        if let Some(s) = session {
                            ui.label(format!("{} · {}", s.label, s.cwd.display()));
                            if s.kind == SessionKind::Editor {
                                ui.colored_label(
                                    appearance::color(&self.theme.status_waiting),
                                    "Unsaved editor buffers remain alive when backgrounded.",
                                );
                                if ui.button("Save all editor buffers").clicked() {
                                    self.send(Request::EditorSave {
                                        session: sid.clone(),
                                    });
                                }
                            }
                        }
                        ui.label("Keep it running in the background, or terminate its processes.");
                        ui.horizontal(|ui| {
                            let keep = ui.button("Keep running");
                            #[cfg(feature = "test-support")]
                            diagnostics::record(ui.ctx(), "close-session-keep", keep.rect);
                            if keep.clicked() {
                                self.remove_tab(&sid);
                                self.close_session = None;
                            }
                            let terminate = ui.button("Terminate");
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                "close-session-terminate",
                                terminate.rect,
                            );
                            if terminate.clicked() {
                                self.send(Request::Stop {
                                    session: sid.clone(),
                                });
                                self.remove_tab(&sid);
                                self.close_session = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.close_session = None;
                            }
                        });
                    });
            }
        }
        if let Some(nid) = self.detail.clone()
            && let Some(note) = self
                .state
                .terminal_notices
                .iter()
                .find(|n| n.id == nid)
                .cloned()
        {
            self.detail = None;
            self.go_session(&note.session_id);
        }
        if let Some(nid) = self.detail.clone()
            && let Some(n) = self
                .state
                .notifications
                .iter()
                .find(|n| n.id == nid)
                .cloned()
            && self.notice_detail_modal_open()
        {
            let session = self
                .state
                .sessions
                .iter()
                .find(|s| s.id == n.session_id)
                .cloned();
            let mut open = true;
            let mut action = AttentionAction::None;
            self.popups
                .window(ctx, "Agent needs attention")
                .id(egui::Id::new("notice-detail"))
                .open(&mut open)
                .default_width(480.0)
                .show(ctx, |ui| {
                    action = attention_card(
                        ui,
                        AttentionCard {
                            theme: &self.theme,
                            notice: &n,
                            session: session.as_ref(),
                            selected: self.active_session.as_ref() == Some(&n.session_id),
                            highlight: true,
                        },
                    );
                });
            self.apply_notice_action(nid, action);
            if !open {
                self.detail = None;
            }
        }
        if self.test_editor && !self.picker_active {
            self.test_editor = false;
            self.picker_active = true;
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(frame)
                .set_title("Test external editor");
            let cwd = self.dialog_directory();
            let settings = self.settings_draft.clone();
            let jobs = self.jobs.clone();
            let tx = self.update_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let path = pollster::block_on(
                    dialog
                        .set_directory(services::existing_directory(cwd))
                        .pick_file(),
                );
                if let Some(path) = path {
                    let _ = jobs.send(Job::TestExternal(
                        path.path().into(),
                        settings.external_editor,
                        settings.external_args,
                    ));
                }
                let _ = tx.send(Update::TestPickerClosed);
                ctx.request_repaint();
            });
        }
        if self.open_path && !self.picker_active {
            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                eprintln!("Fixture native file picker opened");
            }
            self.open_path = false;
            self.picker_active = true;
            let cwd = self.dialog_directory();
            let project = self.selected.clone();
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_parent(frame)
                .set_title("Open file");
            if !self.path_text.is_empty() {
                if let Some(name) = std::path::Path::new(&self.path_text).file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                self.path_text.clear();
            }
            let tx = self.update_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let dialog = dialog.set_directory(services::existing_directory(cwd.clone()));
                let path = pollster::block_on(dialog.pick_file()).map(|p| p.path().to_path_buf());
                let _ = tx.send(Update::PickedFile { path, project, cwd });
                ctx.request_repaint();
            });
        }
        if let Some(sid) = self.search_session.clone() {
            let mut open = true;
            self.popups
                .window(ctx, "Search session history")
                .open(&mut open)
                .default_size([700.0, 500.0])
                .show(ctx, |ui| {
                    ui.text_edit_singleline(&mut self.search);
                    let key = format!("history:{sid}");
                    if !self.texts.contains_key(&key) && self.loading.insert(key.clone()) {
                        let _ = self.jobs.send(Job::rpc(
                            Request::History {
                                session: sid.clone(),
                            },
                            After::Text(key.clone()),
                        ));
                    }
                    if let Some(text) = self.texts.get(&key) {
                        egui::ScrollArea::both().show(ui, |ui| {
                            let needle = self.search.to_lowercase();
                            for (line, text) in text
                                .lines()
                                .enumerate()
                                .filter(|(_, l)| l.to_lowercase().contains(&needle))
                                .take(2000)
                            {
                                ui.monospace(format!("{}  {}", line + 1, text));
                            }
                        });
                    }
                });
            if !open {
                self.search_session = None;
            }
        }
        if self.settings_open {
            self.settings(ctx);
        }
    }
}
