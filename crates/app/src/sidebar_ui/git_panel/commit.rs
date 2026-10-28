//! Commit message, commit options and buttons, and the shelf list.
use super::*;

impl GitPanelPass<'_, '_> {
    pub(super) fn commit_box(&mut self, ui: &mut egui::Ui) {
        let staged = self
            .prepared
            .groups
            .iter()
            .any(|group| group.group == terminator_git::GitGroup::Staged);
        self.commit_message(ui);
        ui.horizontal(|ui| self.commit_options(ui));
        ui.horizontal(|ui| self.commit_buttons(ui, staged));
        self.shelves(ui);
    }

    fn commit_message(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::multiline(self.input.commit_draft)
                .hint_text("Message")
                .desired_width(ui.available_width())
                .desired_rows(4),
        );
        // Subject length guide: Git tools truncate long first lines.
        let subject = self
            .input
            .commit_draft
            .lines()
            .next()
            .map_or(0, |line| line.chars().count());
        if subject > 0 {
            let text = RichText::new(format!("Subject {subject}/72")).small();
            if subject > 72 {
                ui.colored_label(ui.visuals().warn_fg_color, text);
            } else {
                ui.weak(text);
            }
        }
    }

    /// Amend, recent messages, and sign-off.
    fn commit_options(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        let input = &mut *self.input;
        let amend = ui
            .checkbox(input.amend, "Amend")
            .on_hover_text("Amend the last commit (loads its message)");
        if amend.changed() && *input.amend && input.commit_draft.trim().is_empty() {
            self.outcome.load_head_message = true;
        }
        if !input.recent_messages.is_empty() {
            appearance::icon_menu_button(ui, "History", |ui| {
                Self::recent_messages(ui, input.recent_messages, input.commit_draft);
            })
            .response
            .on_hover_text("Recent commit messages");
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-amend", amend.rect);
        let signoff = ui
            .checkbox(input.signoff, "Sign-off")
            .on_hover_text("Add a Signed-off-by trailer");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-signoff", signoff.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = (&amend, &signoff);
    }

    /// Picking a recent message replaces the draft.
    fn recent_messages(ui: &mut egui::Ui, messages: &[String], draft: &mut String) {
        for message in messages {
            let subject = message.lines().next().unwrap_or_default();
            if ui
                .add(egui::Button::new(subject).truncate())
                .on_hover_text(message.as_str())
                .clicked()
            {
                message.clone_into(draft);
                ui.close();
            }
        }
    }

    /// Stage All, Commit, Commit and Push, and Shelve.
    fn commit_buttons(&mut self, ui: &mut egui::Ui, staged: bool) {
        ui.spacing_mut().item_spacing.x = 6.0;
        let input = &*self.input;
        let outcome = &mut *self.outcome;
        let stage_all = ui
            .add_enabled(
                !input.context.changes.is_empty(),
                egui::Button::new("Stage All"),
            )
            .on_hover_text("Stage every change");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-stage-all", stage_all.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = &stage_all;
        if stage_all.clicked() {
            outcome.stage_all = true;
        }
        // Amending with an empty message keeps the previous message.
        let can_commit =
            !input.committing && staged && (*input.amend || !input.commit_draft.trim().is_empty());
        let commit = ui.add_enabled(can_commit, egui::Button::new("Commit"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-commit", commit.rect);
        if commit.clicked() {
            outcome.commit = true;
        }
        let commit_push = ui.add_enabled(can_commit, egui::Button::new("Commit and Push"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-commit-push", commit_push.rect);
        if commit_push.clicked() {
            outcome.commit_push = true;
        }
        if input.committing {
            ui.spinner().on_hover_text("Committing…");
        }
        let shelve = ui
            .add_enabled(staged, egui::Button::new("Shelve"))
            .on_hover_text("Shelve staged changes (git stash)");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-shelve", shelve.rect);
        if shelve.clicked() {
            outcome.shelve = true;
        }
    }

    fn shelves(&mut self, ui: &mut egui::Ui) {
        let shelves = self.input.shelves;
        if shelves.is_empty() {
            return;
        }
        ui.add(
            egui::Label::new(
                RichText::new(format!("SHELVED {}", shelves.len()))
                    .small()
                    .strong(),
            )
            .selectable(false),
        );
        for (reference, message) in shelves {
            ui.horizontal(|ui| self.shelf_row(ui, reference, message));
        }
    }

    fn shelf_row(&mut self, ui: &mut egui::Ui, reference: &str, message: &str) {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::Label::new(message).truncate())
            .on_hover_text(format!("{reference} {message}"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let drop = ui.small_button("Drop");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("git-shelf-drop-{reference}"), drop.rect);
            if drop.clicked() {
                self.outcome.drop_shelf = Some(reference.to_owned());
            }
            let unshelve = ui.small_button("Unshelve");
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("git-shelf-unshelve-{reference}"),
                unshelve.rect,
            );
            if unshelve.clicked() {
                self.outcome.unshelve = Some(reference.to_owned());
            }
        });
    }
}
