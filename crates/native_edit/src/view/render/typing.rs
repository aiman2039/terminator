//! Text input on a focused, editable view: vim Ctrl chords, clipboard
//! pastes, language-server shortcuts, discrete keys and typed (IME) text.
use super::SourceView;
use super::canvas::Canvas;
use super::helpers::RenderHelpers;
use crate::doc::Buffer;
use crate::view::egui_key;
use crate::vim::{Effect, Key as VKey, ModalEngine, Mode};

/// Where one key press went.
enum Routed {
    /// A shortcut ran; egui must not see the key again.
    Consumed,
    /// An engine key, pressed after routing.
    Discrete(VKey),
}

/// One input in arrival order: a routed key press or typed text.
enum Ordered {
    Consumed(egui::Key),
    Key(egui::Key, VKey),
    Text(String),
}

impl<B: Buffer> SourceView<'_, B> {
    /// Press the vim-owned Ctrl chords; returns whether one starts a block
    /// selection.
    pub(super) fn run_vim_ctrls(
        &mut self,
        canvas: &Canvas<'_>,
        vim_ctrls: Vec<(egui::Key, egui::Modifiers, VKey)>,
    ) -> bool {
        // Some backends send both the key and a clipboard event. A
        // block-selection chord must not also insert clipboard text.
        let selecting_block = vim_ctrls.iter().any(|(_, _, key)| *key == VKey::CtrlV);
        for (key, modifiers, vkey) in vim_ctrls {
            self.out
                .effects
                .extend(self.engine.press_key(self.doc, vkey));
            canvas
                .ui
                .input_mut(|input| input.consume_key(modifiers, key));
        }
        selecting_block
    }

    /// Insert clipboard pastes (unless a block chord claimed them), then
    /// drop the paste events: paste is editor-scoped while focused, so
    /// nothing downstream should see it a second time.
    pub(super) fn paste_clipboard(
        &mut self,
        canvas: &Canvas<'_>,
        pastes: Vec<String>,
        selecting_block: bool,
    ) {
        if !selecting_block {
            for text in pastes {
                self.paste_text(&text);
            }
        }
        canvas.ui.input_mut(|input| {
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Paste(_)));
        });
    }

    /// With one clipboard line per caret, each caret gets its own line
    /// (`IntelliJ`); otherwise all get it all.
    fn paste_text(&mut self, text: &str) {
        let carets = self.engine.extra_carets().len().saturating_add(1);
        let lines: Vec<&str> = text.lines().collect();
        let per_caret = carets > 1 && lines.len() == carets;
        let mut order = self.engine.caret_order(&*self.doc).into_iter();
        let vim = self.options.vim;
        self.engine.for_each_caret(self.doc, |engine, doc| {
            // Modeless paste replaces the selection, IDE-style.
            if !vim {
                engine.ide_delete_selection(doc);
            }
            let piece = if per_caret {
                order
                    .next()
                    .and_then(|rank| lines.get(rank))
                    .copied()
                    .unwrap_or_default()
            } else {
                text
            };
            engine.paste_text(doc, piece);
            Vec::new()
        });
    }

    /// Key presses and typed text, in the order they arrived. Printable
    /// chars come from Text events in every mode (so `<`, `"`, `~` and
    /// non-US layouts work); key events only carry non-printable keys.
    pub(super) fn handle_discrete_keys(&mut self, canvas: &Canvas<'_>) {
        let mut consumed = Vec::new();
        for input in self.route_inputs(canvas) {
            match input {
                Ordered::Consumed(key) => consumed.push(key),
                Ordered::Key(key, vkey) => {
                    if self.press_discrete_key(vkey) {
                        consumed.push(key);
                    }
                }
                Ordered::Text(text) => self.take_text(&text),
            }
        }
        // Handled keys must not leak to egui focus/menu handling: arrows
        // stay in the file, Escape keeps editor focus. Typed text was all
        // read here, so stale input never reaches a later frame.
        canvas.ui.input_mut(|input| {
            for key in consumed {
                input.consume_key(egui::Modifiers::NONE, key);
            }
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Text(_)));
        });
    }

    fn route_inputs(&mut self, canvas: &Canvas<'_>) -> Vec<Ordered> {
        let mut ordered = Vec::new();
        // Only key presses and text matter here: skip copying pointer
        // and other events every frame.
        let events: Vec<egui::Event> = canvas.ui.input(|input| {
            input
                .events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)
                    )
                })
                .cloned()
                .collect()
        });
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match self.route_key(canvas, key, modifiers) {
                    Some(Routed::Consumed) => ordered.push(Ordered::Consumed(key)),
                    Some(Routed::Discrete(_))
                        if crate::view::keys::arrives_as_text(key, modifiers) =>
                    {
                        // The Text event types it; outside Insert the key
                        // is still the editor's, not egui's.
                        if self.options.vim && self.engine.mode() != Mode::Insert {
                            ordered.push(Ordered::Consumed(key));
                        }
                    }
                    Some(Routed::Discrete(vkey)) => ordered.push(Ordered::Key(key, vkey)),
                    None => {}
                },
                egui::Event::Text(text) => ordered.push(Ordered::Text(text)),
                _ => {}
            }
        }
        ordered
    }

    fn route_key(
        &mut self,
        canvas: &Canvas<'_>,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> Option<Routed> {
        if self.options.popup_keys && RenderHelpers::popup_key(key) {
            // The host popup reads this key.
            return None;
        }
        if self.options.jetbrains_keys
            && let Some(chord) = crate::view::jetbrains_shortcut(
                key,
                modifiers,
                crate::lsp::server_language(self.language).is_some(),
                cfg!(target_os = "macos"),
            )
        {
            match chord {
                crate::view::JetBrainsKey::Effect(effect) => self.out.effects.push(effect),
                crate::view::JetBrainsKey::GotoLine => self.engine.begin_ex(""),
                crate::view::JetBrainsKey::Problem { backward } => self.jump_to_problem(backward),
            }
            return Some(Routed::Consumed);
        }
        if let Some(effect) = self.shortcut_effect(key, modifiers) {
            self.out.effects.push(effect);
            return Some(Routed::Consumed);
        }
        if key == egui::Key::C
            && modifiers.alt
            && modifiers.shift
            && modifiers.command
            && let Some(reference) = &self.options.reference
        {
            // Cmd/Ctrl+Alt+Shift+C: Copy Reference.
            canvas
                .ui
                .ctx()
                .copy_text(RenderHelpers::caret_reference(reference, self.engine));
            return Some(Routed::Consumed);
        }
        if key == egui::Key::F8 && !modifiers.command && !modifiers.alt {
            self.jump_to_problem(modifiers.shift);
            return Some(Routed::Consumed);
        }
        egui_key(key, modifiers).map(Routed::Discrete)
    }

    /// Language-server and navigation shortcuts that only queue an effect.
    fn shortcut_effect(&self, key: egui::Key, modifiers: egui::Modifiers) -> Option<Effect> {
        let server = crate::lsp::server_language(self.language).is_some();
        let bare = !modifiers.command && !modifiers.ctrl && !modifiers.alt;
        match key {
            // Alt+Enter: show intention actions (`IntelliJ`).
            // Built-in items (Add to agent context) need no server.
            egui::Key::Enter if modifiers.alt && !modifiers.command => Some(Effect::CodeActions),
            // Cmd/Ctrl+Alt+L: reformat code.
            egui::Key::L
                if modifiers.alt && modifiers.command && server && !self.options.read_only =>
            {
                Some(Effect::Format)
            }
            // Ctrl+Space: basic completion (`IntelliJ`).
            egui::Key::Space if modifiers.ctrl && !modifiers.alt && server => {
                Some(Effect::Complete)
            }
            // F12 jumps to the definition under the cursor in every mode
            // (VS convention; F-keys are never vim input and `egui_key`
            // leaves them unmapped).
            egui::Key::F12 if bare => Some(Effect::GotoDefinition),
            // Ctrl+Alt+H: Call Hierarchy (`IntelliJ`).
            egui::Key::H
                if modifiers.ctrl
                    && modifiers.alt
                    && !modifiers.shift
                    && !modifiers.mac_cmd
                    && server =>
            {
                Some(Effect::Hierarchy { types: false })
            }
            // Alt+F7: Find Usages (`IntelliJ`).
            egui::Key::F7 if modifiers.alt && !modifiers.command && server => {
                Some(Effect::FindReferences)
            }
            egui::Key::F2 if server && bare => Some(Effect::RenameSymbol),
            _ => crate::view::ide_navigation_shortcut(key, modifiers, server),
        }
    }

    /// F8 / Shift+F8: next / previous problem, wrapping around.
    fn jump_to_problem(&mut self, backward: bool) {
        let squiggles = &self.options.squiggles;
        let here = (self.engine.cursor().line, self.engine.cursor().col);
        let target = if backward {
            squiggles
                .iter()
                .rev()
                .find(|s| (s.line, s.start) < here)
                .or_else(|| squiggles.last())
        } else {
            squiggles
                .iter()
                .find(|s| (s.line, s.start) > here)
                .or_else(|| squiggles.first())
        };
        if let Some(target) = target {
            self.out.effects.extend(
                self.engine
                    .place_cursor(self.doc, target.line, target.start),
            );
        }
    }

    /// Press one routed (non-printable) key; returns whether egui must
    /// not see it again.
    fn press_discrete_key(&mut self, vkey: VKey) -> bool {
        // Selection indentation also applies with Vim disabled; keep the
        // selected lines instead of opening Insert mode.
        let effects = if self.engine.mode().is_visual() && matches!(vkey, VKey::Tab | VKey::BackTab)
        {
            self.engine
                .for_each_caret(self.doc, |engine, doc| engine.press_key(doc, vkey))
        } else if !self.options.vim && self.engine.mode() != Mode::Command {
            // Modeless: with vim off, no key is ever a vim command.
            // Editing keys edit, and arrows move the IDE caret.
            self.engine
                .for_each_caret(self.doc, |engine, doc| engine.modeless_key(doc, vkey))
        } else {
            let effects = self.engine.press_key(self.doc, vkey);
            self.snapshot_clipboard();
            effects
        };
        self.out.effects.extend(effects);
        true
    }

    /// Typed text: Insert mode types it at every caret; with vim off it
    /// types (replacing a selection); other vim modes read each char as
    /// a command key.
    fn take_text(&mut self, text: &str) {
        let mode = self.engine.mode();
        if mode == Mode::Insert {
            let effects = self
                .engine
                .for_each_caret(self.doc, |engine, doc| engine.type_text(doc, text));
            self.out.effects.extend(effects);
            return;
        }
        if !self.options.vim && mode != Mode::Command {
            for c in text.chars() {
                let effects = self.engine.for_each_caret(self.doc, |engine, doc| {
                    engine.modeless_key(doc, VKey::Char(c))
                });
                self.out.effects.extend(effects);
            }
            return;
        }
        for (at, c) in text.char_indices() {
            if self.engine.mode() == Mode::Insert {
                // An Insert-entering key inside one text event (`ix`):
                // the rest types.
                self.take_text(text.get(at..).unwrap_or_default());
                return;
            }
            self.adopt_clipboard_for_paste(VKey::Char(c));
            let effects = self.engine.press_key(self.doc, VKey::Char(c));
            self.snapshot_clipboard();
            self.out.effects.extend(effects);
        }
    }

    /// Before a Normal-mode `p`/`P`, take the system clipboard into the
    /// unnamed register (`clipboard=unnamed`), so text copied elsewhere
    /// pastes. Read only on that keypress, never per frame.
    fn adopt_clipboard_for_paste(&mut self, vkey: VKey) {
        if self.engine.mode() != Mode::Normal || !self.engine.is_idle() {
            return;
        }
        if let (VKey::Char('p' | 'P'), Some(read)) = (vkey, self.options.read_clipboard)
            && let Some(text) = read()
        {
            self.engine.adopt_clipboard(&text);
        }
    }

    /// After a register-only delete (`d`, `c`, `x`, `s`), note the system
    /// clipboard, so a later `p` pastes the deleted text unless something
    /// new was copied meanwhile.
    fn snapshot_clipboard(&mut self) {
        if self.engine.take_clipboard_snapshot()
            && let Some(read) = self.options.read_clipboard
        {
            self.engine.note_clipboard(read().as_deref());
        }
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod routing_tests;
