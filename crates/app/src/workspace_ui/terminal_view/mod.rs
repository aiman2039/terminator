//! A terminal pane: attach, paint, hover targets, and its menus.
use super::super::*;
use super::hover::link_menu_modifier_pressed;
use super::tabs::cached_terminal_theme;
use super::viewer::Viewer;
use egui_term::TerminalBackend;

mod menu;
mod target;

/// Per-frame state of one painted terminal pane.
struct TerminalFrame<'a> {
    session: &'a Session,
    response: egui::Response,
    input_enabled: bool,
    /// Read only by the test-support scroll evidence.
    #[cfg(feature = "test-support")]
    focused: bool,
    modifier_pressed: bool,
    /// Temp-data slot holding the target key the context menu resolves.
    menu_key: egui::Id,
}

/// Input gates for painting the terminal widget.
#[derive(Clone, Copy)]
struct TerminalGates {
    input_enabled: bool,
    focused: bool,
}

impl Viewer<'_> {
    pub(crate) fn terminal_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let modifier_pressed = link_menu_modifier_pressed(ui.ctx());
        self.app.copy_hovered_terminal_path(ui.ctx());
        self.app.visible_sessions.insert(session.id.clone());
        if self.show_generation_error(ui, session) {
            return;
        }
        self.clear_stable_attach(&session.id);
        if self.show_attach_retry(ui, &session.id) || !self.ensure_terminal_backend(ui, session) {
            return;
        }
        let Some(frame) = self.paint_terminal(ui, session, modifier_pressed) else {
            return;
        };
        let Some(hover) = self.terminal_hover_target(ui, &frame) else {
            return;
        };
        self.request_hover_target(&frame, &hover);
        self.show_resolved_target(ui, &frame, &hover);
        self.terminal_secondary_click(ui, &frame, &hover.token);
        self.terminal_context_menu(&frame);
        self.terminal_hover_popup(ui, &frame);
    }

    /// The session's daemon generation failed: show why, offer to start
    /// the service, and drop its backend. True when shown.
    fn show_generation_error(&mut self, ui: &mut egui::Ui, session: &Session) -> bool {
        let Some(owner) = self
            .app
            .state
            .generations
            .iter()
            .find(|health| health.owner.id == session.generation && health.error.is_some())
        else {
            return false;
        };
        let message = owner.error.clone().unwrap_or_default();
        self.app.backends.remove(&session.id);
        ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
        self.app.start_service_button(ui, true);
        true
    }

    /// An attachment that stayed up past the stable window resets its
    /// retry budget.
    fn clear_stable_attach(&mut self, sid: &String) {
        if self.app.backends.contains_key(sid)
            && self
                .app
                .attach_started
                .get(sid)
                .is_some_and(|started| started.elapsed() >= retry_budget::ATTACH_STABLE_WINDOW)
        {
            self.app.attach_budget.remove(sid);
            self.app.attach_error.remove(sid);
        }
    }

    /// While a detached terminal waits out its retry backoff, show the
    /// error and countdown. True when shown.
    fn show_attach_retry(&mut self, ui: &mut egui::Ui, sid: &String) -> bool {
        if self.app.backends.contains_key(sid) {
            return false;
        }
        let Some(remaining) = self
            .app
            .attach_budget
            .get(sid)
            .and_then(|retry| retry.remaining(Instant::now()))
        else {
            return false;
        };
        let message = self
            .app
            .attach_error
            .get(sid)
            .cloned()
            .unwrap_or_else(|| "Terminal connection closed.".into());
        ui.ctx().request_repaint_after(remaining);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
            ui.label(format!(
                "Reconnecting automatically in {} s…",
                remaining.as_secs().saturating_add(1)
            ));
            if ui.small_button("Retry now").clicked() {
                self.app.attach_budget.remove(sid);
                self.app.attach_error.remove(sid);
                self.app.attach_started.remove(sid);
            }
        });
        true
    }

    /// Attach a backend for the session when it has none. False when the
    /// attach failed this frame (the error is shown).
    fn ensure_terminal_backend(&mut self, ui: &mut egui::Ui, session: &Session) -> bool {
        let sid = &session.id;
        if self.app.backends.contains_key(sid) {
            return true;
        }
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
                let message = format!("Cannot load terminal helper: {e}");
                self.record_attach_failure(sid, &message);
                ui.label(message);
                ui.ctx().request_repaint_after(Duration::from_secs(1));
                return false;
            }
        };
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(self.app.state.settings.font_size),
        });
        let size =
            egui_term::TerminalSize::from_pane(ui.available_size(), font.font_measure(ui.ctx()));
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
                true
            }
            Err(e) => {
                let message = format!("Cannot attach terminal: {e}");
                self.record_attach_failure(sid, &message);
                ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
                ui.ctx().request_repaint_after(Duration::from_secs(1));
                false
            }
        }
    }

    fn record_attach_failure(&mut self, sid: &str, message: &str) {
        self.app
            .attach_budget
            .entry(sid.to_owned())
            .or_default()
            .record_failure(Instant::now());
        self.app
            .attach_error
            .insert(sid.to_owned(), message.to_owned());
    }

    /// Find bar, copy, image paste, then the terminal widget itself.
    fn paint_terminal<'a>(
        &mut self,
        ui: &mut egui::Ui,
        session: &'a Session,
        modifier_pressed: bool,
    ) -> Option<TerminalFrame<'a>> {
        let sid = &session.id;
        self.terminal_find_bar(ui, sid);
        let input_enabled = self.terminal_input_for(sid);
        let find_open = self.app.terminal_find.contains_key(sid);
        if !find_open && self.terminal_copy_for(sid) {
            self.app.copy_terminal_selection(ui.ctx(), sid);
        }
        let focused = input_enabled
            && !find_open
            && self.app.active_session.as_ref() == Some(sid)
            && self
                .app
                .markdown
                .entries
                .get(sid)
                .is_none_or(|p| p.editor_focused);
        self.paste_clipboard_images(ui, sid, focused);
        let gates = TerminalGates {
            input_enabled,
            focused,
        };
        let response = self.add_terminal_widget(ui, sid, gates)?;
        #[cfg(feature = "test-support")]
        Self::record_terminal_rects(ui, session, &response);
        if input_enabled && response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            self.app.terminal_pressed(ui.ctx(), sid);
        }
        Some(TerminalFrame {
            session,
            response,
            input_enabled,
            #[cfg(feature = "test-support")]
            focused,
            modifier_pressed,
            menu_key: egui::Id::unique(("terminal-menu-target", sid.as_str())),
        })
    }

    fn paste_clipboard_images(&mut self, ui: &mut egui::Ui, sid: &str, focused: bool) {
        if !focused {
            return;
        }
        let count =
            ui.input_mut(|input| clipboard::take_image_paste(&mut input.events, input.modifiers));
        for _ in 0..count {
            let _ = self.app.jobs.send(Job::PasteClipboard(sid.to_owned()));
        }
    }

    fn add_terminal_widget(
        &mut self,
        ui: &mut egui::Ui,
        sid: &str,
        gates: TerminalGates,
    ) -> Option<egui::Response> {
        let find_paint = self.find_paint_for(sid);
        let backend = self.app.backends.get_mut(sid)?;
        backend.set_painted(true);
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(self.app.state.settings.font_size),
        });
        let theme = cached_terminal_theme(&mut self.app.terminal_theme, &self.app.theme);
        let view = TerminalView::new(ui, backend)
            .external_links(true)
            .set_theme(theme)
            .set_focus(gates.focused)
            .set_font(font)
            .set_size(ui.available_size())
            .find_highlight(find_paint);
        Some(ui.add_enabled(gates.input_enabled, view))
    }

    #[cfg(feature = "test-support")]
    fn record_terminal_rects(ui: &egui::Ui, session: &Session, response: &egui::Response) {
        if session.kind == SessionKind::Shell {
            diagnostics::record(ui.ctx(), "terminal", response.rect);
        }
        diagnostics::record(ui.ctx(), &format!("terminal:{}", session.id), response.rect);
        if session.kind == SessionKind::Editor {
            diagnostics::record(ui.ctx(), "editor-terminal", response.rect);
        }
    }
}
