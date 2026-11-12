//! Modal rendering.
mod pickers;
mod session_close;
#[cfg(test)]
mod tests;
mod worktree_remove;
pub(crate) use worktree_remove::RemoveCheck;

use crate::{AttentionAction, AttentionCard, attention_card, *};

impl App {
    fn first_project_dialog(&mut self, ctx: &egui::Context) {
        if !(cfg!(target_os = "macos")
            && self
                .preferences
                .needs_setup(self.state_loaded, self.state.projects.len())
            && !self.picker.busy())
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
                ui.vertical_centered(|ui| {
                    if appearance::primary_button(ui, "Choose project folder").clicked() {
                        self.request_pick(PickKind::Project);
                    }
                    ui.add_space(4.0);
                    if ui.link("Skip").clicked() {
                        self.preferences.setup_completed = true;
                    }
                });
            });
    }

    pub(crate) fn modals(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.first_project_dialog(ctx);
        self.native_close_modal(ctx);
        self.layout_save_dialog(ctx);
        self.host_key_dialog(ctx);
        self.restart_confirm_dialog(ctx);
        self.open_requested_picker(frame);
        if self.close_workspace.is_none() && self.close_session.is_none() {
            self.idle_close_fallback = None;
        }
        self.poll_workspace_close(ctx);
        self.advance_session_close_queue();
        self.close_session_modal(ctx);
        self.open_terminal_notice_detail();
        self.notice_detail_modal(ctx);
        self.session_peek_modal(ctx);
        self.worktree_remove_dialog(ctx);
        self.ask_agent_dialog(ctx);
        self.init_command_dialog(ctx);
        self.missing_offer_dialog(ctx);
    }

    fn restart_confirm_dialog(&mut self, ctx: &egui::Context) {
        if self.service_op != ServiceOp::ConfirmingRestart {
            return;
        }
        let live = self
            .state
            .sessions
            .iter()
            .filter(|session| session.lifecycle.live())
            .count();
        let failed = appearance::color(&self.theme.status_failed);
        let window = self.popups.window(ctx, "Restart session service?");
        let dialog = appearance::Confirm::new(
            "",
            "Terminator will quit and reopen. Saved history is kept, but stopped sessions cannot be restored.",
        )
        .cancel(appearance::ConfirmButton::new("Cancel").id("cancel-restart-session"))
        .destructive(
            appearance::ConfirmButton::new("Stop all sessions and restart")
                .id("confirm-restart-session"),
        );
        let answer = appearance::confirm(ctx, window, dialog, |ui| {
            let sessions = appearance::count_noun(live, "live session");
            for line in [
                format!("{sessions} will be stopped."),
                "Unsaved editor changes will be lost. Running shells, agents, and jobs will stop."
                    .to_owned(),
            ] {
                ui.label(RichText::new(line).strong().color(failed));
            }
        });
        match answer {
            Some(appearance::ConfirmAnswer::Destructive) => self.begin_session_restart(),
            Some(_) => self.cancel_session_restart(),
            None => {}
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
        if !self.notice_detail_open() {
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
                        pinned: self.session_pinned(&n.session_id),
                        triage: self.triage_available(),
                        group_extra: &[],
                    },
                );
            });
        self.apply_notice_action(nid, action);
        if !open {
            self.detail = None;
        }
    }

    pub(crate) fn layout_save_dialog(&mut self, ctx: &egui::Context) {
        if self.layout_save_name.is_none() {
            return;
        }
        let mut name = self.layout_save_name.take().unwrap_or_default();
        let mut done = false;
        self.popups
            .centered(ctx, "Save layout")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(280.0);
                ui.label("Name this layout:");
                let field = ui.add(
                    appearance::singleline(&mut name)
                        .hint_text("My layout")
                        .desired_width(f32::INFINITY),
                );
                let mut action = None;
                // Only Enter typed in the field saves: the Enter that chose
                // the palette row must not save an empty name.
                if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    action = Some(true);
                } else if ui.memory(|memory| memory.focused().is_none()) {
                    field.request_focus();
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
                            self.preferences
                                .save_current_layout(&trimmed, self.project_width);
                        }
                    }
                    done = true;
                    ui.close();
                }
            });
        // The dialog stays open until Save or Cancel.
        if !done {
            self.layout_save_name = Some(name);
        }
    }

    pub(crate) fn save_layout_dialog(&mut self) {
        self.layout_save_name = Some(String::new());
    }
}
