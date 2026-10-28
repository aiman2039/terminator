use super::super::super::*;
use super::super::viewer::Viewer;
use super::TerminalFrame;
use crate::app::TerminalActionInput;
use egui_term::TerminalBackend;

impl Viewer<'_> {
    pub(super) fn terminal_context_menu(&mut self, frame: &TerminalFrame<'_>) {
        let session = frame.session;
        let sid = &session.id;
        appearance::context_menu(&frame.response, |ui| {
            let cwd = crate::remote_ui::location_label(session);
            appearance::target_header(ui, &cwd, &cwd);
            let selected = self
                .app
                .backends
                .get(sid)
                .map_or_else(String::new, TerminalBackend::selectable_content);
            self.terminal_clipboard_items(ui, sid, &selected);
            ui.separator();
            let key = Tab::Terminal(sid.clone()).key();
            let pane = self.pane_for(&key);
            self.app
                .new_terminal_menu(super::super::menus::NewTerminalMenuInput {
                    ui,
                    pane,
                    strip: self.strip,
                    window: self.window,
                });
            ui.separator();
            self.terminal_target_items(ui, frame, &selected);
            ui.separator();
            self.terminal_session_items(ui, session);
            ui.separator();
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            if appearance::menu_item(
                ui,
                "Close session…",
                "X",
                &self.app.shortcut_label(Action::CloseSession),
            )
            .clicked()
            {
                self.app.close_session = Some(sid.clone());
                ui.close();
            }
        });
    }

    fn terminal_clipboard_items(&mut self, ui: &mut egui::Ui, sid: &String, selected: &str) {
        let command = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl+Shift+"
        };
        ui.add_enabled_ui(!selected.is_empty(), |ui| {
            if appearance::menu_item(ui, "Copy", "Copy", &format!("{command}C")).clicked() {
                ui.ctx().copy_text(selected.to_owned());
                ui.close();
            }
        });
        if appearance::menu_item(
            ui,
            "Select all",
            "TextSelect",
            &self.app.shortcut_label(Action::SelectAll),
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
    }

    /// File actions for the right-clicked target, and Open file path.
    fn terminal_target_items(
        &mut self,
        ui: &mut egui::Ui,
        frame: &TerminalFrame<'_>,
        selected: &str,
    ) {
        let key = ui
            .ctx()
            .data(|d| d.get_temp::<String>(frame.menu_key))
            .unwrap_or_default();
        if let Some(Some(target)) = self.app.targets.get(&key).cloned() {
            appearance::target_header(ui, &target.compact(), &target.display());
            if let Some(action) = file_actions::menu(ui, file_actions::target_menu(&target)) {
                self.app.terminal_action(TerminalActionInput {
                    ctx: ui.ctx(),
                    session: frame.session,
                    target: &target,
                    action,
                });
            }
        }
        if appearance::menu_item(
            ui,
            "Open file path…",
            "File",
            &self.app.shortcut_label(Action::OpenFile),
        )
        .clicked()
        {
            selected.clone_into(&mut self.app.path_text);
            self.app.open_path = true;
            ui.close();
        }
    }

    /// Scrollback search, editor save/compare, and copy directory.
    fn terminal_session_items(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        if appearance::menu_item(
            ui,
            "Search scrollback",
            "Search",
            &self.app.shortcut_label(Action::SearchScrollback),
        )
        .clicked()
        {
            self.app.open_scrollback_search(ui.ctx(), sid);
            ui.close();
        }
        if session.kind == SessionKind::Editor && !session.review {
            self.terminal_editor_items(ui, sid);
        }
        if appearance::menu_item(
            ui,
            if session.remote.is_some() {
                "Copy project directory"
            } else {
                "Copy working directory"
            },
            "Folder",
            &self.app.shortcut_label(Action::CopyWorkingDirectory),
        )
        .clicked()
        {
            ui.ctx().copy_text(
                session
                    .remote
                    .as_ref()
                    .map_or_else(|| session.cwd.display().to_string(), |r| r.root.clone()),
            );
            ui.close();
        }
    }

    fn terminal_editor_items(&mut self, ui: &mut egui::Ui, sid: &str) {
        if appearance::menu_item(
            ui,
            "Save all",
            "Save",
            &self.app.shortcut_label(Action::EditorSave),
        )
        .clicked()
        {
            self.app.send(Request::EditorSave {
                session: sid.to_owned(),
            });
            ui.close();
        }
        if appearance::menu_item(
            ui,
            "Compare disk",
            "FileDiff",
            &self.app.shortcut_label(Action::CompareDisk),
        )
        .clicked()
        {
            self.app.send(Request::EditorCompare {
                session: sid.to_owned(),
            });
            ui.close();
        }
    }

    /// The pinned file menu opened from a hovered terminal link.
    pub(super) fn terminal_hover_popup(&mut self, ui: &egui::Ui, frame: &TerminalFrame<'_>) {
        let session = frame.session;
        let sid = &session.id;
        let Some(popup) = self.app.hover_popup.clone() else {
            return;
        };
        if popup.session != *sid {
            return;
        }
        let mut open = true;
        egui::Popup::from_response(&frame.response)
            .id(egui::Id::unique((
                "terminal-hover",
                sid.as_str(),
                popup.key.as_str(),
            )))
            .anchor(popup.rect)
            .open_bool(&mut open)
            .style(appearance::menu_style)
            .show(|ui| {
                ui.set_max_width(440.0);
                ui.horizontal(Self::hover_popup_title);
                appearance::target_header(ui, &popup.target.compact(), &popup.target.display());
                if let Some(action) =
                    file_actions::menu(ui, file_actions::target_menu(&popup.target))
                {
                    self.app.terminal_action(TerminalActionInput {
                        ctx: ui.ctx(),
                        session,
                        target: &popup.target,
                        action,
                    });
                }
            });
        if !open {
            self.app.dismiss_hover_popup(ui.ctx());
        }
    }

    fn hover_popup_title(ui: &mut egui::Ui) {
        ui.label("File actions");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = appearance::sidebar_action(ui, "X", "Dismiss file menu");
            #[cfg(feature = "test-support")]
            diagnostics::record_ui(ui, "terminal-file-menu-close", close.rect);
            if close.clicked() {
                ui.close();
            }
        });
    }
}
