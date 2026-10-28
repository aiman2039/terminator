//! Keyboard shortcuts on a focused, editable view: multi-caret chords,
//! IDE clipboard/history actions, and IDE navigation and line edits.
use super::SourceView;
use super::canvas::Canvas;
use super::helpers::{CaretChord, RenderHelpers};
use crate::doc::Buffer;
use crate::highlight::Language;
use crate::view::{
    IdeAction, IdeEdit, comment_token, ctrl_h_is_motion, ctrl_v_is_block, ide_edit_in_vim,
    ide_edit_key, ide_shortcut,
};
use crate::vim::ModalEngine;
use crate::vim::{Effect, IdeMotion, Key as VKey, Mode, VimEngine};

type KeyChord = (egui::Key, egui::Modifiers);

/// Shortcut presses and clipboard events read in one pass over input.
#[derive(Default)]
struct ShortcutBatch {
    actions: Vec<(egui::Key, egui::Modifiers, IdeAction)>,
    /// Ctrl chords vim owns in its current mode, as engine keys.
    vim_ctrls: Vec<(egui::Key, egui::Modifiers, VKey)>,
    pastes: Vec<String>,
}

struct EditCaretInput<'a, B> {
    engine: &'a mut VimEngine,
    doc: &'a mut B,
    edit: IdeEdit,
    language: Language,
}

impl<B: Buffer> SourceView<'_, B> {
    /// Multiple carets (`IntelliJ`): Add / Unselect / Select All
    /// Occurrences, and Escape back to one caret. Modeless only.
    pub(super) fn handle_caret_chords(&mut self, canvas: &Canvas<'_>, focused: bool) {
        if !focused || self.options.read_only || self.options.vim {
            return;
        }
        for ((key, modifiers), chord) in self.caret_chords(canvas) {
            match chord {
                CaretChord::AddNext => {
                    self.engine.add_next_occurrence(self.doc);
                }
                CaretChord::RemoveLast => self.engine.remove_last_occurrence(),
                CaretChord::AddAll => self.engine.add_all_occurrences(self.doc),
                CaretChord::Collapse => self.engine.clear_extra_carets(),
            }
            canvas.ui.input_mut(|input| {
                input.consume_key(modifiers, key);
                // An Alt chord may also arrive as typed text.
                input.events.retain(|event| {
                    !matches!(event, egui::Event::Text(text) if text.chars().count() == 1 && key != egui::Key::Escape && modifiers.alt)
                });
            });
        }
    }

    fn caret_chords(&self, canvas: &Canvas<'_>) -> Vec<(KeyChord, CaretChord)> {
        let mac = cfg!(target_os = "macos");
        let mut chords = Vec::new();
        canvas.ui.input(|input| {
            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };
                if let Some(chord) = RenderHelpers::caret_chord(mac, *key, *modifiers) {
                    chords.push(((*key, *modifiers), chord));
                } else if *key == egui::Key::Escape
                    && !self.engine.extra_carets().is_empty()
                    && !(self.options.popup_keys && RenderHelpers::popup_key(*key))
                {
                    chords.push(((*key, *modifiers), CaretChord::Collapse));
                }
            }
        });
        chords
    }

    /// IDE shortcuts first: copy/cut/paste/select/undo work in every mode
    /// (IntelliJ-style), then plain vim keys. Ctrl-V and Ctrl-H detour to
    /// vim where it owns them.
    pub(super) fn handle_keyboard(&mut self, canvas: &Canvas<'_>, focused: bool) {
        if !focused || self.options.read_only {
            return;
        }
        let mut batch = self.read_shortcuts(canvas);
        self.route_ctrl_h(&mut batch);
        let ShortcutBatch {
            actions,
            vim_ctrls,
            pastes,
        } = batch;
        self.run_ide_actions(canvas, actions);
        self.run_ide_edits(canvas);
        let selecting_block = self.run_vim_ctrls(canvas, vim_ctrls);
        self.paste_clipboard(canvas, pastes, selecting_block);
        self.handle_discrete_keys(canvas);
        // Keep repainting while focused so the caret stays live. The
        // caret does not blink, so a slow tick is enough: a per-frame
        // repaint keeps the whole GUI awake for nothing.
        canvas
            .ui
            .ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }

    fn read_shortcuts(&self, canvas: &Canvas<'_>) -> ShortcutBatch {
        let mut batch = ShortcutBatch::default();
        canvas.ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => self.classify_shortcut((*key, *modifiers), &mut batch),
                    egui::Event::Paste(text) => batch.pastes.push(text.clone()),
                    _ => {}
                }
            }
        });
        batch
    }

    fn classify_shortcut(&self, (key, modifiers): KeyChord, batch: &mut ShortcutBatch) {
        // Control is independent of Command on macOS. Ctrl-Q is Vim's
        // block-selection alias for backends that reserve Ctrl-V for
        // clipboard paste.
        if matches!(key, egui::Key::V | egui::Key::Q)
            && modifiers.ctrl
            && !modifiers.mac_cmd
            && !modifiers.alt
            && !modifiers.shift
            && ctrl_v_is_block(self.options.vim, self.engine.mode())
        {
            batch.vim_ctrls.push((key, modifiers, VKey::CtrlV));
        } else if let Some(action) = ide_shortcut(key, modifiers) {
            batch.actions.push((key, modifiers, action));
        }
    }

    /// Vim-owned Ctrl chords become engine keys before IDE actions run,
    /// so the mode they observe is current.
    fn route_ctrl_h(&self, batch: &mut ShortcutBatch) {
        let ShortcutBatch {
            actions, vim_ctrls, ..
        } = batch;
        actions.retain(|(key, modifiers, action)| {
            let mode = self.engine.mode();
            match action {
                IdeAction::Replace
                    if *key == egui::Key::H && ctrl_h_is_motion(self.options.vim, mode) =>
                {
                    vim_ctrls.push((*key, *modifiers, VKey::Backspace));
                    false
                }
                _ => true,
            }
        });
    }

    fn run_ide_actions(
        &mut self,
        canvas: &Canvas<'_>,
        actions: Vec<(egui::Key, egui::Modifiers, IdeAction)>,
    ) {
        for (key, modifiers, action) in actions {
            self.run_ide_action(canvas, action);
            canvas
                .ui
                .input_mut(|input| input.consume_key(modifiers, key));
        }
    }

    fn run_ide_action(&mut self, canvas: &Canvas<'_>, action: IdeAction) {
        match action {
            IdeAction::Copy => {
                let text = self.caret_copy_text();
                if !text.is_empty() {
                    canvas.ui.ctx().copy_text(text);
                }
            }
            IdeAction::Cut => {
                let text = self.cut_carets();
                if !text.is_empty() {
                    canvas.ui.ctx().copy_text(text);
                }
            }
            IdeAction::SelectAll => {
                self.engine.clear_extra_carets();
                self.engine.select_all(self.doc);
            }
            IdeAction::Undo => {
                self.engine.clear_extra_carets();
                self.engine.undo(self.doc);
            }
            IdeAction::Redo => {
                self.engine.clear_extra_carets();
                self.engine.redo(self.doc);
            }
            IdeAction::PasteKey => {}
            IdeAction::Save => self.out.effects.push(Effect::Save),
            IdeAction::Find if !self.options.vim => {
                self.out.effects.push(Effect::FindBar { replace: false });
            }
            IdeAction::Replace if !self.options.vim => {
                self.out.effects.push(Effect::FindBar { replace: true });
            }
            IdeAction::Find => self.engine.begin_search(),
            IdeAction::Replace => self.engine.begin_ex("%s/"),
            IdeAction::Hover => self.out.effects.push(Effect::Hover),
        }
    }

    /// The selection, or one line per caret with extra carets.
    fn caret_copy_text(&self) -> String {
        if self.engine.extra_carets().is_empty() {
            self.engine.copy_text(&*self.doc)
        } else {
            self.engine.caret_texts(&*self.doc).join("\n")
        }
    }

    fn cut_carets(&mut self) -> String {
        if self.engine.extra_carets().is_empty() {
            return self.engine.cut(self.doc);
        }
        let texts = self.engine.caret_texts(&*self.doc).join("\n");
        self.engine.for_each_caret(self.doc, |engine, doc| {
            engine.ide_delete_selection(doc);
            Vec::new()
        });
        texts
    }

    /// IDE navigation and line edits (`IntelliJ` keymap): Shift selects,
    /// word/line/document motions, duplicate, move, comment. Vim keeps its
    /// own plain and word keys.
    fn run_ide_edits(&mut self, canvas: &Canvas<'_>) {
        if self.engine.mode() == Mode::Command {
            return;
        }
        for (key, modifiers, edit) in self.read_ide_edits(canvas) {
            if matches!(edit, IdeEdit::Fold { .. } | IdeEdit::Unfold { .. }) {
                self.out.fold_actions.push(edit);
            } else {
                self.apply_ide_edit(edit);
            }
            canvas
                .ui
                .input_mut(|input| input.consume_key(modifiers, key));
        }
    }

    fn read_ide_edits(&self, canvas: &Canvas<'_>) -> Vec<(egui::Key, egui::Modifiers, IdeEdit)> {
        let ui = canvas.ui;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Viewport height in rows."
        )]
        let page_rows = (ui.clip_rect().height() / self.geometry.row_height)
            .floor()
            .max(1.0) as usize;
        let mut edits = Vec::new();
        ui.input(|input| {
            for event in &input.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && !(self.options.popup_keys && RenderHelpers::popup_key(*key))
                    && let Some(edit) =
                        ide_edit_key(*key, *modifiers, cfg!(target_os = "macos"), page_rows)
                    && (!self.options.vim || ide_edit_in_vim(edit, *modifiers))
                {
                    edits.push((*key, *modifiers, edit));
                }
            }
        });
        edits
    }

    /// Every caret runs the edit (one undo step for all).
    fn apply_ide_edit(&mut self, edit: IdeEdit) {
        let language = self.language;
        let vertical_move = &mut self.out.vertical_move;
        self.engine.for_each_caret(self.doc, |engine, doc| {
            if let Some(down) = Self::edit_caret(EditCaretInput {
                engine,
                doc,
                edit,
                language,
            }) {
                *vertical_move = Some(down);
            }
            Vec::new()
        });
    }

    /// Apply `edit` at the engine's current caret; a vertical move reports
    /// its direction (`true` = down).
    fn edit_caret(input: EditCaretInput<'_, B>) -> Option<bool> {
        let EditCaretInput {
            engine,
            doc,
            edit,
            language,
        } = input;
        match edit {
            IdeEdit::DeleteWordBack => engine.ide_delete_word_back(doc),
            IdeEdit::DeleteLines => engine.ide_delete_lines(doc),
            IdeEdit::Duplicate => engine.ide_duplicate(doc),
            IdeEdit::MoveLines { up } => engine.ide_move_lines(doc, up),
            IdeEdit::ToggleComment => {
                if let Some(token) = comment_token(language) {
                    engine.ide_toggle_comment(doc, token);
                }
            }
            IdeEdit::JoinLines => engine.ide_join_lines(doc),
            IdeEdit::Fold { .. } | IdeEdit::Unfold { .. } => {}
            IdeEdit::Move(
                motion @ (IdeMotion::Up
                | IdeMotion::Down
                | IdeMotion::PageUp(_)
                | IdeMotion::PageDown(_)),
                select,
            ) => {
                engine.ide_move(doc, motion, select);
                return Some(matches!(motion, IdeMotion::Down | IdeMotion::PageDown(_)));
            }
            IdeEdit::Move(motion, select) => engine.ide_move(doc, motion, select),
            IdeEdit::ExtendSelection => engine.ide_extend_selection(doc),
            IdeEdit::ShrinkSelection => engine.ide_shrink_selection(doc),
        }
        None
    }
}
