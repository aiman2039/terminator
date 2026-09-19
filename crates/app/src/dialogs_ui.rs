//! Modal rendering.
use super::{AttentionAction, AttentionCard, attention_card, *};

impl App {
    fn first_project_dialog(&mut self, ctx: &egui::Context) {
        if !(cfg!(target_os = "macos")
            && self
                .preferences
                .needs_setup(self.state_loaded, self.state.projects.len())
            && !self.picker_active)
        {
            return;
        }
        self.popups
            .centered(ctx, "Choose your first project")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label("Choose a folder to begin. You can add more projects later.");
                ui.add_space(8.0);
                let width = ui.available_width();
                if ui
                    .add_sized([width, 28.0], egui::Button::new("Choose project folder"))
                    .clicked()
                {
                    self.add_project = true;
                }
                if ui
                    .add_sized([width, 28.0], egui::Button::new("Skip"))
                    .clicked()
                {
                    self.preferences.setup_completed = true;
                }
            });
    }

    pub(super) fn modals(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.first_project_dialog(ctx);
        if self.restart_confirm {
            let live = self
                .state
                .sessions
                .iter()
                .filter(|session| session.lifecycle.live())
                .count();
            let mut open = true;
            self.popups
                .window(ctx, "Restart session service?")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!("All {live} live session(s) will be stopped."))
                            .strong()
                            .color(appearance::color(&self.theme.status_failed)),
                    );
                    ui.label(
                        RichText::new("Unsaved editor changes will be lost. Running shells, agents, and jobs will stop.")
                            .strong()
                            .color(appearance::color(&self.theme.status_failed)),
                    );
                    ui.add_space(8.0);
                    ui.label("Terminator will quit and reopen. Saved history is kept, but stopped sessions cannot be restored.");
                    ui.horizontal(|ui| {
                        let confirm = ui.button("Stop all sessions and restart");
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "confirm-restart-session", confirm.rect);
                        if confirm.clicked() {
                            self.begin_session_restart();
                        }
                        let cancel = ui.button("Cancel");
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "cancel-restart-session", cancel.rect);
                        if cancel.clicked() {
                            self.restart_confirm = false;
                        }
                    });
                });
            if !open {
                self.restart_confirm = false;
            }
        }
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
            let service = self.services.clone();
            self.services.dialog(async move {
                let cwd = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(services::existing_directory(cwd))
                    })
                    .await?;
                let dialog = dialog.set_directory(cwd);
                let path = dialog.pick_folder().await.map(|p| p.path().to_path_buf());
                service
                    .emit(Update::PickedProject(path, generation))
                    .await?;
                Ok(())
            });
        }
        if self.close_workspace.is_none() && self.close_session.is_none() {
            self.idle_close_fallback = None;
        }
        self.poll_workspace_close(ctx);
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
            } else if self
                .check_idle_close(editor_close::Target::Pane(sid.clone()), vec![sid.clone()])
            {
                // Confirmation follows only if daemon verification cannot close safely.
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
            let service = self.services.clone();
            self.services.dialog(async move {
                let cwd = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(services::existing_directory(cwd))
                    })
                    .await?;
                let path = dialog.set_directory(cwd).pick_file().await;
                if let Some(path) = path {
                    let _ = jobs.send(Job::TestExternal(
                        path.path().into(),
                        settings.external_editor,
                        settings.external_args,
                    ));
                }
                service.emit(Update::TestPickerClosed).await?;
                Ok(())
            });
        }
        if self.pick_audio && !self.picker_active {
            self.pick_audio = false;
            self.picker_active = true;
            let cwd = self.dialog_directory();
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(frame)
                .set_title("Add audio files")
                .add_filter("Audio", player::AUDIO_EXTENSIONS);
            let service = self.services.clone();
            self.services.dialog(async move {
                let cwd = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(services::existing_directory(cwd))
                    })
                    .await?;
                let dialog = dialog.set_directory(cwd);
                let paths = dialog
                    .pick_files()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|file| file.path().to_path_buf())
                    .collect();
                service.emit(Update::PickedAudio(paths)).await?;
                Ok(())
            });
        }
        if self.pick_audio_dir && !self.picker_active {
            self.pick_audio_dir = false;
            self.picker_active = true;
            let cwd = self.dialog_directory();
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(frame)
                .set_title("Add audio directory");
            let service = self.services.clone();
            self.services.dialog(async move {
                let cwd = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(services::existing_directory(cwd))
                    })
                    .await?;
                let dialog = dialog.set_directory(cwd);
                let folder = dialog.pick_folder().await.map(|f| f.path().to_path_buf());
                let paths = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(folder
                            .map(|path| player::audio_from_dir(&path))
                            .unwrap_or_default())
                    })
                    .await?;
                service.emit(Update::PickedAudio(paths)).await?;
                Ok(())
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
            let service = self.services.clone();
            self.services.dialog(async move {
                let selected = cwd.clone();
                let directory = service
                    .fs()
                    .run(&async_service::CancellationToken::new(), move || {
                        Ok(services::existing_directory(selected))
                    })
                    .await?;
                let dialog = dialog.set_directory(directory);
                let path = dialog.pick_file().await.map(|p| p.path().to_path_buf());
                service
                    .emit(Update::PickedFile { path, project, cwd })
                    .await?;
                Ok(())
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
        if self.palette_open {
            self.palette(ctx);
        }
        if self.worktree_draft.is_some() {
            self.worktree_wizard(ctx);
        }
        self.worktree_remove_dialog(ctx);
        if let Some(target) = self.browse_target.take() {
            if self.picker_active {
                self.browse_target = Some(target);
            } else {
                self.picker_active = true;
                let folder = matches!(target, BrowseTarget::WorktreeDest);
                let dialog =
                    rfd::AsyncFileDialog::new()
                        .set_parent(frame)
                        .set_title(match target {
                            BrowseTarget::Shell => "Choose shell",
                            BrowseTarget::Editor => "Choose editor",
                            BrowseTarget::External => "Choose external editor",
                            BrowseTarget::WorktreeDest => "Choose parent folder",
                        });
                let cwd = self.dialog_directory();
                let service = self.services.clone();
                self.services.dialog(async move {
                    let cwd = service
                        .fs()
                        .run(&async_service::CancellationToken::new(), move || {
                            Ok(services::existing_directory(cwd))
                        })
                        .await?;
                    let dialog = dialog.set_directory(cwd);
                    let path = if folder {
                        dialog.pick_folder().await.map(|p| p.path().to_path_buf())
                    } else {
                        dialog.pick_file().await.map(|p| p.path().to_path_buf())
                    };
                    service.emit(Update::PickedPath { path, target }).await?;
                    Ok(())
                });
            }
        }
    }

    pub(super) fn poll_workspace_close(&mut self, ctx: &egui::Context) {
        loop {
            let Some((project, tab_id)) = self.close_workspace.clone() else {
                return;
            };
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
            self.confirm_workspace_close(ctx, &project, &tab_id, live);
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
                self.state
                    .sessions
                    .iter()
                    .any(|session| &session.id == id && session.lifecycle.live())
            })
            .collect()
    }

    fn confirm_workspace_close(
        &mut self,
        ctx: &egui::Context,
        project: &str,
        tab_id: &str,
        live: Vec<String>,
    ) {
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
