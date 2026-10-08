use super::super::*;
use super::header::{HOVER_POPUP_DELAY, hover_popup_wake, should_replace_hover_popup};
use super::tabs::cached_terminal_theme;
use super::viewer::Viewer;
use egui_term::TerminalBackend;
impl Viewer<'_> {
    pub(crate) fn terminal_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        self.app.visible_sessions.insert(sid.clone());
        if !self.app.backends.contains_key(sid)
            && self
                .app
                .attach_budget
                .get(sid)
                .is_some_and(|budget| budget.exhausted(&session.cwd, 0))
        {
            let message = self.app.attach_error.get(sid).cloned().unwrap_or_else(|| {
                "Stopped retrying this terminal after repeated attachment failures. Check the session service, then retry."
                    .into()
            });
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
                if ui.small_button("Retry").clicked() {
                    self.app.attach_budget.remove(sid);
                    self.app.attach_error.remove(sid);
                    self.app.attach_started.remove(sid);
                }
            });
            return;
        }
        if !self.app.backends.contains_key(sid) {
            let id = self.app.next_backend;
            self.app.next_backend = self.app.next_backend.saturating_add(1);
            let owner = self
                .app
                .state
                .generations
                .iter()
                .find(|g| g.owner.id == session.generation);
            let endpoint = owner
                .map(|g| g.owner.paths())
                .unwrap_or_else(|| self.app.paths.clone());
            let helper_result = owner
                .and_then(|g| g.helper.clone())
                .map(Ok)
                .unwrap_or_else(|| installation::attachment_helper(&self.app.state));
            let helper = match helper_result {
                Ok(p) => p,
                Err(e) => {
                    ui.label(e.to_string());
                    return;
                }
            };
            let font = egui_term::TerminalFont::new(egui_term::FontSettings {
                font_type: egui::FontId::monospace(self.app.state.settings.font_size),
            });
            let size = egui_term::TerminalSize::from_pane(
                ui.available_size(),
                font.font_measure(ui.ctx()),
            );
            match TerminalBackend::new(
                id,
                ui.ctx().clone(),
                self.app.pty_tx.clone(),
                egui_term::BackendSettings {
                    shell: helper.to_string_lossy().into(),
                    args: vec![
                        "attach".into(),
                        sid.clone(),
                        endpoint.data.to_string_lossy().into(),
                        endpoint.runtime.to_string_lossy().into(),
                    ],
                    working_directory: None,
                    size,
                },
            ) {
                Ok(b) => {
                    self.app.attach_started.insert(sid.clone(), Instant::now());
                    self.app.backends.insert(sid.clone(), b);
                    self.app.backend_ids.insert(id, sid.clone());
                }
                Err(e) => {
                    let message = format!("Cannot attach terminal: {e}");
                    self.app
                        .attach_budget
                        .entry(sid.clone())
                        .or_default()
                        .record(&session.cwd, 0, true);
                    self.app.attach_error.insert(sid.clone(), message.clone());
                    ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
                    return;
                }
            }
        }
        self.terminal_find_bar(ui, sid);
        let input_enabled = if self.strip {
            self.app.strip_terminal_input_enabled(sid)
        } else {
            self.app.terminal_input_enabled(sid)
        };
        let find_open = self.app.terminal_find.contains_key(sid);
        let focused = input_enabled
            && !find_open
            && self.app.active_session.as_ref() == Some(sid)
            && self
                .app
                .markdown
                .entries
                .get(sid)
                .is_none_or(|p| p.editor_focused);
        if focused {
            let count = ui
                .input_mut(|input| clipboard::take_image_paste(&mut input.events, input.modifiers));
            for _ in 0..count {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
            }
        }
        let find_paint = self.find_paint_for(sid);
        let Some(backend) = self.app.backends.get_mut(sid) else {
            return;
        };
        backend.set_painted(true);
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(self.app.state.settings.font_size),
        });
        let theme = cached_terminal_theme(&mut self.app.terminal_theme, &self.app.theme);
        let view = TerminalView::new(ui, backend)
            .external_links(true)
            .set_theme(theme)
            .set_focus(focused)
            .set_font(font)
            .set_size(ui.available_size())
            .find_highlight(find_paint);
        let response = ui.add_enabled(input_enabled, view);
        #[cfg(feature = "test-support")]
        {
            if session.kind == SessionKind::Shell {
                diagnostics::record(ui.ctx(), "terminal", response.rect);
            }
            diagnostics::record(ui.ctx(), &format!("terminal:{sid}"), response.rect);
            if session.kind == SessionKind::Editor {
                diagnostics::record(ui.ctx(), "editor-terminal", response.rect);
            }
        }
        if input_enabled && response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            self.app.terminal_pressed(ui.ctx(), sid);
        }
        let dragging = ui.input(|i| i.pointer.any_down());
        let (mouse_reporting, target, dismissal_token) = {
            let Some(backend) = self.app.backends.get(sid) else {
                return;
            };
            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                let content = backend.last_content();
                let snapshot = (
                    focused,
                    content.display_offset,
                    content.terminal_mode.bits(),
                );
                let key = egui::Id::new(("scroll-evidence", sid));
                if ui.ctx().data(|d| d.get_temp::<(bool, usize, u32)>(key)) != Some(snapshot) {
                    eprintln!(
                        "Scroll evidence: session={sid} focused={} offset={} modes={}",
                        snapshot.0, snapshot.1, snapshot.2
                    );
                    ui.ctx().data_mut(|d| d.insert_temp(key, snapshot));
                }
            }
            let mouse_reporting = backend
                .last_content()
                .terminal_mode
                .intersects(egui_term::TerminalMode::MOUSE_MODE);
            let target = if !input_enabled || dragging || self.app.hover_popup.is_some() {
                None
            } else {
                response.hover_pos().and_then(|pos| {
                    backend.target_at(pos.x - response.rect.left(), pos.y - response.rect.top())
                })
            };
            // A closing popup can still own egui's hover hit-test for this pass.
            // Use the grid position to decide whether the pointer left its file.
            let dismissal_token = self
                .app
                .dismissed_hover
                .as_ref()
                .filter(|(session, _)| session == sid)
                .map(|_| {
                    ui.ctx()
                        .pointer_hover_pos()
                        .filter(|pos| response.rect.contains(*pos))
                        .and_then(|pos| {
                            backend.target_at(
                                pos.x - response.rect.left(),
                                pos.y - response.rect.top(),
                            )
                        })
                        .map_or_else(String::new, |target| target.text)
                });
            (mouse_reporting, target, dismissal_token)
        };
        let token = target.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), token);
        let dismissal_key = dismissal_token
            .map(|token| format!("target:{}:{}:{}", sid, session.cwd.display(), token));
        let dismissed = self.app.hover_popup.is_none()
            && self
                .app
                .hover_target_dismissed(sid, dismissal_key.as_deref().unwrap_or(&key));
        if !token.is_empty()
            && !self.app.targets.contains_key(&key)
            && self.app.loading.insert(key.clone())
        {
            let _ = self.app.jobs.send(Job::ResolveTarget(
                key.clone(),
                token.clone(),
                session.cwd.clone(),
            ));
        }
        let resolved = self.app.targets.get(&key).cloned().flatten();
        if let (Some(target), Some(resolved)) = (&target, &resolved) {
            if !ui.input(|i| i.pointer.any_down()) && !mouse_reporting {
                for rect in &target.rects {
                    let rect = rect.translate(response.rect.min.to_vec2());
                    ui.painter().with_clip_rect(response.rect).line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        egui::Stroke::new(1.0, appearance::color(&self.app.theme.accent)),
                    );
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let popup_key = self.app.hover_popup.as_ref().map(|popup| popup.key.clone());
                if !dismissed
                    && should_replace_hover_popup(
                        &mut self.app.hover,
                        &key,
                        popup_key.as_deref(),
                        HOVER_POPUP_DELAY,
                    )
                {
                    let rect = target
                        .rects
                        .first()
                        .copied()
                        .unwrap_or(egui::Rect::ZERO)
                        .translate(response.rect.min.to_vec2());
                    self.app.hover_popup = Some(HoverPopup {
                        session: sid.clone(),
                        key: key.clone(),
                        target: resolved.clone(),
                        rect,
                    });
                    #[cfg(feature = "test-support")]
                    eprintln!("Terminal file menu opened: {}", resolved.display());
                }
                if !dismissed
                    && let Some(wake) = hover_popup_wake(
                        &self.app.hover,
                        &key,
                        self.app
                            .hover_popup
                            .as_ref()
                            .map(|popup| popup.key.as_str()),
                        HOVER_POPUP_DELAY,
                    )
                {
                    ui.ctx().request_repaint_after(wake);
                }
            }
        } else if response.contains_pointer() {
            self.app.hover = None;
        }
        if !mouse_reporting
            && response.clicked()
            && ui.input(|i| {
                if cfg!(target_os = "macos") {
                    i.modifiers.mac_cmd
                } else {
                    i.modifiers.ctrl
                }
            })
        {
            if let Some(target) = &resolved {
                self.app
                    .terminal_action(ui.ctx(), session, target, FileAction::Open);
            } else if !token.is_empty() {
                self.app.pending_target_action = Some((key.clone(), session.clone()));
            }
        }
        let menu_key = egui::Id::new(("terminal-menu-target", sid.as_str()));
        if response.secondary_clicked() {
            let selected = self
                .app
                .backends
                .get(sid)
                .map_or_else(String::new, TerminalBackend::selectable_content);
            let text = if selected.trim().is_empty() {
                token.clone()
            } else {
                selected
            };
            let key = format!("target:{}:{}:{}", sid, session.cwd.display(), text);
            ui.ctx().data_mut(|d| d.insert_temp(menu_key, key.clone()));
            if !self.app.targets.contains_key(&key) && self.app.loading.insert(key.clone()) {
                let _ = self
                    .app
                    .jobs
                    .send(Job::ResolveTarget(key, text, session.cwd.clone()));
            }
        }
        appearance::context_menu(&response, |ui| {
            let cwd = session.cwd.display().to_string();
            appearance::target_header(ui, &cwd, &cwd);
            let selected = self
                .app
                .backends
                .get(sid)
                .map_or_else(String::new, TerminalBackend::selectable_content);
            let command = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+Shift+"
            };
            ui.add_enabled_ui(!selected.is_empty(), |ui| {
                if appearance::menu_item(ui, "Copy", "Copy", &format!("{command}C")).clicked() {
                    ui.ctx().copy_text(selected.clone());
                    ui.close();
                }
            });
            if appearance::menu_item(
                ui,
                "Select all",
                "TextSelect",
                &self.app.shortcut_label("select_all"),
            )
            .clicked()
            {
                if let Some(backend) = self.app.backends.get_mut(sid) {
                    backend.select_all();
                }
                ui.close();
            }
            if appearance::menu_item(ui, "Paste", "Clipboard", &format!("{command}V")).clicked() {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
                ui.close();
            }
            ui.separator();
            let key = Tab::Terminal(sid.clone()).key();
            let pane = self.pane_for(&key);
            self.app
                .new_terminal_menu(ui, pane, self.strip, self.window);
            ui.separator();
            let key = ui
                .ctx()
                .data(|d| d.get_temp::<String>(menu_key))
                .unwrap_or_default();
            if let Some(Some(target)) = self.app.targets.get(&key).cloned() {
                appearance::target_header(ui, &target.compact(), &target.display());
                if let Some(action) = file_actions::menu(ui, file_actions::target_menu(&target)) {
                    self.app.terminal_action(ui.ctx(), session, &target, action);
                }
            }
            if appearance::menu_item(
                ui,
                "Open file path…",
                "File",
                &self.app.shortcut_label("open_file"),
            )
            .clicked()
            {
                self.app.path_text.clone_from(&selected);
                self.app.open_path = true;
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(
                ui,
                "Search scrollback",
                "Search",
                &self.app.shortcut_label("search_scrollback"),
            )
            .clicked()
            {
                self.app.open_scrollback_search(ui.ctx(), sid);
                ui.close();
            }
            if session.kind == SessionKind::Editor && !session.review {
                if appearance::menu_item(
                    ui,
                    "Save all",
                    "Save",
                    &self.app.shortcut_label("editor_save"),
                )
                .clicked()
                {
                    self.app.send(Request::EditorSave {
                        session: sid.clone(),
                    });
                    ui.close();
                }
                if appearance::menu_item(
                    ui,
                    "Compare disk",
                    "FileDiff",
                    &self.app.shortcut_label("compare_disk"),
                )
                .clicked()
                {
                    self.app.send(Request::EditorCompare {
                        session: sid.clone(),
                    });
                    ui.close();
                }
            }
            if appearance::menu_item(
                ui,
                "Copy working directory",
                "Folder",
                &self.app.shortcut_label("copy_working_directory"),
            )
            .clicked()
            {
                ui.ctx().copy_text(session.cwd.display().to_string());
                ui.close();
            }
            ui.separator();
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            if appearance::menu_item(
                ui,
                "Close session…",
                "X",
                &self.app.shortcut_label("close_session"),
            )
            .clicked()
            {
                self.app.close_session = Some(sid.clone());
                ui.close();
            }
        });
        if let Some(popup) = self.app.hover_popup.clone()
            && popup.session == *sid
        {
            let mut open = true;
            egui::Popup::from_response(&response)
                .id(egui::Id::new((
                    "terminal-hover",
                    sid.as_str(),
                    popup.key.as_str(),
                )))
                .anchor(popup.rect)
                .open_bool(&mut open)
                .style(appearance::menu_style)
                .show(|ui| {
                    ui.set_max_width(440.0);
                    ui.horizontal(|ui| {
                        ui.label("File actions");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let close = appearance::sidebar_action(ui, "X", "Dismiss file menu");
                            #[cfg(feature = "test-support")]
                            diagnostics::record(ui.ctx(), "terminal-file-menu-close", close.rect);
                            if close.clicked() {
                                ui.close();
                            }
                        });
                    });
                    appearance::target_header(ui, &popup.target.compact(), &popup.target.display());
                    if let Some(action) =
                        file_actions::menu(ui, file_actions::target_menu(&popup.target))
                    {
                        self.app
                            .terminal_action(ui.ctx(), session, &popup.target, action);
                    }
                });
            if !open {
                self.app.dismiss_hover_popup(ui.ctx());
            }
        }
    }
}
