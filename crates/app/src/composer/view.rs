//! The composer window. Keys typed here go to its text area: the window's
//! text field holds egui focus, so `terminal_has_keyboard` is false and no
//! terminal reads them. Cmd+Enter (Paste and send) and Tab (take the
//! highlighted file) are consumed before the text area sees them.
use super::{Composer, insert_mention, mention_at, queue_ready};
use crate::{App, appearance};
use eframe::egui;
use std::path::PathBuf;
use terminator_native_edit::finder::QueryScoredInInput;

/// `@` completion rows shown at most.
const MENTION_ROWS: usize = 8;

/// What the user asked for this frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ComposerAct {
    None,
    Paste,
    PasteSend,
    Queue,
    SendNext,
    Unqueue(usize),
    Recall(String),
    Mention(PathBuf),
}

/// The text field's id, so tests and the caret code find it.
pub(crate) fn text_id() -> egui::Id {
    egui::Id::unique("prompt-composer-text")
}

impl App {
    pub(crate) fn composer_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut open) = self.composer.open.take() else {
            return;
        };
        if !self.ask_targets().iter().any(|(id, _)| *id == open.target) {
            self.info = Some("The agent is no longer running. Nothing was sent.".into());
            return;
        }
        let mut shown = true;
        let mut act = ComposerAct::None;
        self.popups
            .window(ctx, "Prompt composer")
            .open(&mut shown)
            .collapsible(false)
            .show(ctx, |ui| act = self.composer_form(ui, &mut open));
        let escape = ctx.memory(|m| m.has_focus(text_id()))
            && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if self.apply_composer_act(ctx, &mut open, act) && shown && !escape {
            self.composer.open = Some(open);
        }
    }

    /// Runs `act`. False when the window closes (the prompt went).
    pub(crate) fn apply_composer_act(
        &mut self,
        ctx: &egui::Context,
        open: &mut Composer,
        act: ComposerAct,
    ) -> bool {
        let target = open.target.clone();
        match act {
            ComposerAct::None => {}
            ComposerAct::Paste | ComposerAct::PasteSend => {
                let enter = act == ComposerAct::PasteSend;
                if self.composer_paste(&target, &open.text, enter) {
                    return false;
                }
            }
            ComposerAct::Queue => {
                self.queue_prompt(&target, &open.text);
                open.text.clear();
            }
            ComposerAct::SendNext => {
                self.send_next_queued(&target);
            }
            ComposerAct::Unqueue(index) => {
                if let Some(queue) = self.composer.queues.get_mut(&target) {
                    queue.remove(index);
                }
            }
            ComposerAct::Recall(text) => open.text = text,
            ComposerAct::Mention(path) => self.take_mention(ctx, open, &path),
        }
        true
    }

    /// Replaces the `@word` at the caret by the picked file.
    fn take_mention(&self, ctx: &egui::Context, open: &mut Composer, path: &std::path::Path) {
        let Some((start, _)) = mention_at(&open.text, open.caret) else {
            return;
        };
        let root = self.session_root(&open.target);
        let (text, caret) = insert_mention(&open.text, (start, open.caret), path, root.as_deref());
        open.text = text;
        open.caret = caret;
        open.pick = 0;
        open.focus = true;
        let chars = open
            .text
            .get(..caret)
            .map_or(0, |head| head.chars().count());
        if let Some(mut state) = egui::TextEdit::load_state(ctx, text_id()) {
            let at = egui::text::CCursor::new(chars);
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(at)));
            state.store(ctx, text_id());
        }
    }

    /// Files matching the `@word` at the caret, from the file index.
    pub(crate) fn mention_matches(&self, open: &Composer) -> Vec<PathBuf> {
        let Some((_, query)) = mention_at(&open.text, open.caret) else {
            return Vec::new();
        };
        let root = self.session_root(&open.target);
        self.file_index
            .query_scored_in(QueryScoredInInput {
                root: root.as_deref(),
                exclude: &[],
                pattern: &query,
                limit: MENTION_ROWS,
            })
            .into_iter()
            .map(|(_, file)| file.path)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn composer_form_for_test(
        &mut self,
        ui: &mut egui::Ui,
        open: &mut Composer,
    ) -> ComposerAct {
        self.composer_form(ui, open)
    }

    fn composer_form(&mut self, ui: &mut egui::Ui, open: &mut Composer) -> ComposerAct {
        self.refresh_file_index();
        let label = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == open.target)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        let state = self.hook_agent_state(&open.target);
        ui.weak(format!(
            "To {label} · {}",
            state.map_or("state unknown", terminator_core::AgentState::label)
        ));
        let matches = self.mention_matches(open);
        let mut act = composer_keys(ui, open, &matches);
        let output = egui::TextEdit::multiline(&mut open.text)
            .id(text_id())
            .hint_text("Write a prompt… Type @ to add a file.")
            .desired_rows(6)
            .desired_width(520.0)
            .show(ui);
        if std::mem::take(&mut open.focus) {
            output.response.request_focus();
        }
        if let Some(range) = output.cursor_range {
            open.caret = byte_offset(&open.text, range.primary.index.into());
        }
        if let Some(path) = mention_rows(ui, &matches, open.pick) {
            act = ComposerAct::Mention(path);
        }
        if let Some(queued) = self.queue_rows(ui, &open.target, state) {
            act = queued;
        }
        if let Some(footer) = self.composer_footer(ui, open) {
            act = footer;
        }
        act
    }

    /// The follow-up queue and its "Send next" button.
    fn queue_rows(
        &self,
        ui: &mut egui::Ui,
        target: &str,
        state: Option<terminator_core::AgentState>,
    ) -> Option<ComposerAct> {
        let queue = self.composer.queues.get(target).filter(|q| !q.is_empty())?;
        let mut act = None;
        ui.add_space(6.0);
        ui.weak(format!("Queued ({})", queue.len()));
        for (index, prompt) in queue.iter().enumerate() {
            ui.horizontal(|ui| {
                if appearance::sidebar_action(ui, "X", "Remove from the queue").clicked() {
                    act = Some(ComposerAct::Unqueue(index));
                }
                ui.add(egui::Label::new(crate::diff::notes::one_line(prompt)).truncate());
            });
        }
        let ready = queue_ready(state);
        let why = (!ready).then_some("Waits until the agent asks for input or completes");
        let send = appearance::enabled_unless(ui, why, |ui| ui.button("Send next"));
        if send.clicked() && ready {
            act = Some(ComposerAct::SendNext);
        }
        act
    }

    fn composer_footer(&self, ui: &mut egui::Ui, open: &Composer) -> Option<ComposerAct> {
        let history = self
            .session_root(&open.target)
            .and_then(|root| self.preferences.prompt_history.get(&root).cloned())
            .unwrap_or_default();
        let empty = open.text.trim().is_empty();
        let mut act = None;
        appearance::footer_row(ui, |ui| {
            let why = empty.then_some("Write a prompt first");
            if appearance::enabled_unless(ui, why, |ui| ui.button("Paste and send")).clicked() {
                act = Some(ComposerAct::PasteSend);
            }
            if appearance::enabled_unless(ui, why, |ui| ui.button("Paste")).clicked() {
                act = Some(ComposerAct::Paste);
            }
            if appearance::enabled_unless(ui, why, |ui| ui.button("Add to queue")).clicked() {
                act = Some(ComposerAct::Queue);
            }
            let _ = appearance::icon_menu_button(ui, "History", |ui| {
                if history.is_empty() {
                    ui.weak("No prompts yet in this project.");
                }
                for prompt in &history {
                    let line = crate::diff::notes::one_line(prompt);
                    if ui.add(egui::Button::new(line).truncate()).clicked() {
                        act = Some(ComposerAct::Recall(prompt.clone()));
                        ui.close();
                    }
                }
            });
        });
        act
    }
}

/// Keys the composer takes before its text area: Cmd+Enter sends; with
/// `@` matches shown, Tab takes the highlighted file and Up/Down move.
fn composer_keys(ui: &mut egui::Ui, open: &mut Composer, matches: &[PathBuf]) -> ComposerAct {
    if !ui.memory(|m| m.has_focus(text_id())) {
        return ComposerAct::None;
    }
    let command = egui::Modifiers::COMMAND;
    if ui.input_mut(|i| i.consume_key(command, egui::Key::Enter)) && !open.text.trim().is_empty() {
        return ComposerAct::PasteSend;
    }
    if matches.is_empty() {
        return ComposerAct::None;
    }
    let none = egui::Modifiers::NONE;
    let (tab, up, down) = ui.input_mut(|i| {
        (
            i.consume_key(none, egui::Key::Tab),
            i.consume_key(none, egui::Key::ArrowUp),
            i.consume_key(none, egui::Key::ArrowDown),
        )
    });
    let last = matches.len().saturating_sub(1);
    if down {
        open.pick = open.pick.saturating_add(1).min(last);
    }
    if up {
        open.pick = open.pick.saturating_sub(1);
    }
    match matches.get(open.pick.min(last)) {
        Some(path) if tab => ComposerAct::Mention(path.clone()),
        _ => ComposerAct::None,
    }
}

/// The `@` completion list. Returns the clicked file.
fn mention_rows(ui: &mut egui::Ui, matches: &[PathBuf], pick: usize) -> Option<PathBuf> {
    let mut clicked = None;
    for (index, path) in matches.iter().enumerate() {
        let name = path.display().to_string();
        if ui
            .add(egui::Button::selectable(index == pick, name).truncate())
            .clicked()
        {
            clicked = Some(path.clone());
        }
    }
    if !matches.is_empty() {
        ui.weak("Tab or click adds the file.");
    }
    clicked
}

/// The byte offset of char index `chars` in `text`.
fn byte_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(at, _)| at)
}
