//! Modal rendering.
mod pickers;
mod session_close;
#[cfg(test)]
mod tests;

use crate::{AttentionAction, AttentionCard, attention_card, *};

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

    pub(crate) fn modals(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.first_project_dialog(ctx);
        self.native_close_modal(ctx);
        self.layout_save_dialog(ctx);
        self.host_key_dialog(ctx);
        self.restart_confirm_dialog(ctx);
        self.add_project_picker(frame);
        if self.close_workspace.is_none() && self.close_session.is_none() {
            self.idle_close_fallback = None;
        }
        self.poll_workspace_close(ctx);
        self.close_session_modal(ctx);
        self.open_terminal_notice_detail();
        self.notice_detail_modal(ctx);
        self.test_editor_picker(frame);
        self.audio_files_picker(frame);
        self.audio_directory_picker(frame);
        self.open_file_picker(frame);
        self.worktree_remove_dialog(ctx);
        self.browse_target_picker(frame);
    }

    fn restart_confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.restart_confirm {
            return;
        }
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
                ui.horizontal(|ui| self.restart_confirm_buttons(ui));
            });
        if !open {
            self.restart_confirm = false;
        }
    }

    fn restart_confirm_buttons(&mut self, ui: &mut egui::Ui) {
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
    }

    fn open_terminal_notice_detail(&mut self) {
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
    }

    fn notice_detail_modal(&mut self, ctx: &egui::Context) {
        let Some(nid) = self.detail.clone() else {
            return;
        };
        let Some(n) = self
            .state
            .notifications
            .iter()
            .find(|n| n.id == nid)
            .cloned()
        else {
            return;
        };
        if !self.notice_detail_modal_open() {
            return;
        }
        let session = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == n.session_id)
            .cloned();
        let mut open = true;
        let mut action = AttentionAction::None;
        let presented = self.present_session(&n.session_id);
        let history = self.notice_history_brand(&n);
        let brand_icon = history
            .as_ref()
            .map(|(icon, _)| *icon)
            .or(presented.brand_icon);
        let history_label = history.map(|(_, label)| label);
        let brand_label = history_label
            .as_deref()
            .or(presented.brand_label.as_deref());
        self.popups
            .window(ctx, "Agent needs attention")
            .id(egui::Id::unique("notice-detail"))
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
                        brand_icon,
                        brand_label,
                        show_read: false,
                        group_extra: &[],
                    },
                );
            });
        self.apply_notice_action(nid, action);
        if !open {
            self.detail = None;
        }
    }

    fn layout_save_dialog(&mut self, ctx: &egui::Context) {
        if self.layout_save_name.is_none() {
            return;
        }
        let mut name = self.layout_save_name.take().unwrap_or_default();
        self.popups
            .centered(ctx, "Save layout")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(280.0);
                ui.label("Name this layout:");
                ui.add(
                    appearance::singleline(&mut name)
                        .hint_text("My layout")
                        .desired_width(f32::INFINITY),
                );
                let mut action = None;
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    action = Some(true);
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        action = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(false);
                    }
                });
                if let Some(save) = action {
                    if save {
                        let trimmed = name.trim().to_string();
                        if !trimmed.is_empty() {
                            self.preferences.save_current_layout(&trimmed);
                        }
                    }
                    ui.close();
                }
            });
    }

    pub(crate) fn save_layout_dialog(&mut self) {
        self.layout_save_name = Some(String::new());
    }
}
