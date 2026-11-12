//! The Commit tab body: the changes fill the top and the commit area sits
//! at the bottom (`JetBrains` style); and the Stash tab's shelf list.
use super::*;

/// Commit-area height before its first paint is measured.
const COMMIT_AREA_ESTIMATE: f32 = 220.0;
/// The changes keep at least this much height above the commit area.
const MIN_TREE_HEIGHT: f32 = 80.0;
/// The commit message field starts at this many rows and grows with its
/// text up to [`MESSAGE_MAX_ROWS`], then scrolls.
const MESSAGE_ROWS: usize = 2;
const MESSAGE_MAX_ROWS: f32 = 8.0;
/// Git tools truncate a longer subject line.
const SUBJECT_LIMIT: usize = 72;
/// The key that runs Commit from the message field.
const COMMIT_KEYS: &str = if cfg!(target_os = "macos") {
    "⌘↩"
} else {
    "Ctrl+Enter"
};

/// What decides whether Commit can run now.
pub(super) struct CommitBlockInput<'a> {
    pub(super) running: bool,
    pub(super) staged: bool,
    pub(super) amend: bool,
    pub(super) draft: &'a str,
}

/// Why Commit and Commit and Push are disabled, or `None` when they can
/// run. Amending with an empty message keeps the previous message.
pub(super) fn commit_block_reason(input: CommitBlockInput<'_>) -> Option<&'static str> {
    if input.running {
        Some("Wait for the commit to finish")
    } else if !input.staged {
        Some("Stage a file first")
    } else if !input.amend && input.draft.trim().is_empty() {
        Some("Write a commit message")
    } else {
        None
    }
}

/// Characters in the draft's first line.
fn subject_length(draft: &str) -> usize {
    draft.lines().next().map_or(0, |line| line.chars().count())
}

impl GitPanelPass<'_, '_> {
    /// Changes above, the commit area below. The commit area is laid out
    /// at its height from the last paint; a change of height repaints.
    pub(super) fn commit_tab(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let id = ui.scope_id().with("git-commit-area");
        let last = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(id))
            .unwrap_or(COMMIT_AREA_ESTIMATE);
        let split = (full.bottom() - last).max(full.top() + MIN_TREE_HEIGHT);
        let tree = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), split - 4.0));
        let mut tree_ui = ui.new_child(egui::UiBuilder::new().max_rect(tree));
        self.changes_area(&mut tree_ui);
        let area = egui::Rect::from_min_max(egui::pos2(full.left(), split), full.max);
        let mut area_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        self.commit_area(&mut area_ui);
        let height = area_ui.min_rect().height();
        if (height - last).abs() > 0.5 {
            ui.ctx().data_mut(|data| data.insert_temp(id, height));
            ui.ctx().request_discard("Git commit area height changed");
        }
        ui.allocate_rect(full, egui::Sense::hover());
    }

    /// A refresh error, then the changes (or "Working tree clean").
    fn changes_area(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.input.context.error {
            git_error_row(ui, self.input.theme, error, self.outcome);
        }
        if self.input.context.changes.is_empty() {
            ui.weak("Working tree clean");
        }
        appearance::sidebar_scroll("git").show(ui, |ui| self.changes(ui));
    }

    /// Options, the message, the buttons, then a running commit and the
    /// last failure.
    fn commit_area(&mut self, ui: &mut egui::Ui) {
        let staged = self
            .prepared
            .groups
            .iter()
            .any(|group| group.group == terminator_git::GitGroup::Staged);
        let blocked = commit_block_reason(CommitBlockInput {
            running: self.input.commit_run.is_some(),
            staged,
            amend: *self.input.amend,
            draft: self.input.commit_draft,
        });
        ui.separator();
        ui.horizontal(|ui| self.commit_options(ui));
        self.commit_message(ui, blocked);
        self.readiness_line(ui);
        ui.horizontal_wrapped(|ui| self.commit_buttons(ui, blocked));
        self.commit_progress(ui);
        self.commit_failure(ui);
    }

    /// The message: two rows that grow with the text to eight, then
    /// scroll. Cmd+Enter (Ctrl+Enter) in it runs Commit when it can run.
    fn commit_message(&mut self, ui: &mut egui::Ui, blocked: Option<&str>) {
        let id = ui.make_persistent_id("git-commit-message");
        // Taken before the field reads keys, so it never adds a line.
        if ui.memory(|memory| memory.has_focus(id))
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
            && blocked.is_none()
        {
            self.outcome.commit = true;
        }
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        let margin = 2.0 * ui.spacing().button_padding.y;
        let message = egui::ScrollArea::vertical()
            .id_salt("git-commit-message")
            .max_height(row * MESSAGE_MAX_ROWS + margin)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(self.input.commit_draft)
                        .id(id)
                        .hint_text("Commit Message")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width())
                        .desired_rows(MESSAGE_ROWS),
                )
            })
            .inner;
        if self.input.focus_message {
            message.request_focus();
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
        // Subject length guide at the row's end.
        let subject = subject_length(input.commit_draft);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = RichText::new(format!("{subject}/{SUBJECT_LIMIT}")).small();
            if subject > SUBJECT_LIMIT {
                ui.colored_label(ui.visuals().warn_fg_color, text)
            } else {
                ui.weak(text)
            }
            .on_hover_text("Subject length; Git tools truncate a longer first line");
        });
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

    /// Commit (the main action) and Commit and Push on one row; each
    /// disabled one says why on hover.
    fn commit_buttons(&mut self, ui: &mut egui::Ui, blocked: Option<&str>) {
        ui.spacing_mut().item_spacing.x = 6.0;
        let outcome = &mut *self.outcome;
        let commit =
            appearance::enabled_unless(ui, blocked, |ui| appearance::primary_button(ui, "Commit"))
                .on_hover_text(appearance::shortcut_tip("Commit", COMMIT_KEYS));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-commit", commit.rect);
        if commit.clicked() {
            outcome.commit = true;
        }
        let commit_push =
            appearance::enabled_unless(ui, blocked, |ui| ui.button("Commit and Push"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-commit-push", commit_push.rect);
        if commit_push.clicked() {
            outcome.commit_push = true;
        }
    }

    /// The Stash tab: every shelf with Unshelve and Drop.
    pub(super) fn stash_tab(&mut self, ui: &mut egui::Ui) {
        let shelves = self.input.shelves;
        appearance::sidebar_scroll("git-stash").show(ui, |ui| {
            if shelves.is_empty() {
                ui.weak("No shelved changes");
            }
            for (reference, message) in shelves {
                ui.horizontal(|ui| self.shelf_row(ui, reference, message));
            }
        });
    }

    /// One shelf with Drop and Unshelve, which a commit running in this
    /// repository disables.
    fn shelf_row(&mut self, ui: &mut egui::Ui, reference: &str, message: &str) {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(16.0, GIT_TREE_ROW_HEIGHT), egui::Sense::hover());
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect,
            icon: "Archive",
            size: appearance::TOOLBAR_ICON,
            tint: appearance::icon_color(),
        });
        ui.add(egui::Label::new(message).truncate())
            .on_hover_text(format!(
                "{} {message}",
                crate::sidebar_ui::explorer_prompts::short_hash(reference)
            ));
        let lock = self.input.commit_lock.stash;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let drop = appearance::enabled_unless(ui, lock, |ui| ui.small_button("Drop"));
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("git-shelf-drop-{reference}"), drop.rect);
            if drop.clicked() {
                self.outcome.drop_shelf = Some(reference.to_owned());
            }
            let unshelve = appearance::enabled_unless(ui, lock, |ui| ui.small_button("Unshelve"));
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

/// A refresh error with Retry.
pub(super) fn git_error_row(
    ui: &mut egui::Ui,
    theme: &AppearanceConfig,
    error: &str,
    outcome: &mut GitPanelOutcome,
) {
    ui.horizontal(|ui| {
        ui.colored_label(appearance::color(&theme.status_failed), error);
        if ui.small_button("Retry").clicked() {
            outcome.refresh = true;
        }
    });
}
