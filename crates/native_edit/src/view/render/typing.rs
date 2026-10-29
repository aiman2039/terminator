//! Text input on a focused, editable view: vim Ctrl chords, clipboard
//! pastes, language-server shortcuts, discrete keys and typed (IME) text.
use super::SourceView;
use super::canvas::Canvas;
use super::helpers::RenderHelpers;
use crate::doc::Buffer;
use crate::view::{echo_of_handled_key, egui_key};
use crate::vim::{Effect, Key as VKey, ModalEngine, Mode};

/// Where one key press went.
enum Routed {
    /// A shortcut ran; egui must not see the key again.
    Consumed,
    /// An engine key, pressed after routing.
    Discrete(VKey),
}

/// Key presses sorted for the engine and for consumption.
#[derive(Default)]
struct RoutedKeys {
    discrete: Vec<(egui::Key, VKey)>,
    consumed: Vec<egui::Key>,
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

    /// Discrete keys (edge-triggered pressed events), then typed text.
    pub(super) fn handle_discrete_keys(&mut self, canvas: &Canvas<'_>) {
        let RoutedKeys {
            discrete,
            mut consumed,
        } = self.route_keys(canvas);
        // Printable chars handled below as discrete vim keys; their Text
        // echo must not type a second time (the `i` in `i`).
        let mut handled_chars: Vec<char> = Vec::new();
        for (key, vkey) in discrete {
            if self.press_discrete_key(vkey, &mut handled_chars) {
                consumed.push(key);
            }
        }
        // Handled keys must not leak to egui focus/menu handling: arrows
        // stay in the file, Escape keeps editor focus.
        canvas.ui.input_mut(|input| {
            for key in consumed {
                input.consume_key(egui::Modifiers::NONE, key);
            }
        });
        self.take_typed_text(canvas, handled_chars);
    }

    fn route_keys(&mut self, canvas: &Canvas<'_>) -> RoutedKeys {
        let mut routed = RoutedKeys::default();
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
                match self.route_key(canvas, *key, *modifiers) {
                    Some(Routed::Consumed) => routed.consumed.push(*key),
                    Some(Routed::Discrete(vkey)) => routed.discrete.push((*key, vkey)),
                    None => {}
                }
            }
        });
        routed
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
            egui::Key::Enter if modifiers.alt && !modifiers.command && server => {
                Some(Effect::CodeActions)
            }
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

    /// Press one routed key; returns whether egui must not see it again.
    fn press_discrete_key(&mut self, vkey: VKey, handled_chars: &mut Vec<char>) -> bool {
        // Single printable chars in Insert mode arrive as Text events
        // below; the key event only carries control keys.
        if self.engine.mode() == Mode::Insert && matches!(vkey, VKey::Char(_)) {
            return false;
        }
        if let VKey::Char(c) = vkey {
            handled_chars.push(c);
        }
        // Selection indentation also applies with Vim disabled; keep the
        // selected lines instead of opening Insert mode.
        let effects = if self.engine.mode().is_visual() && matches!(vkey, VKey::Tab | VKey::BackTab)
        {
            self.engine
                .for_each_caret(self.doc, |engine, doc| engine.press_key(doc, vkey))
        } else if !self.options.vim && self.engine.mode() != Mode::Command {
            // Modeless: with vim off, no key is ever a vim command.
            // Printable keys type (replacing a selection), editing keys
            // edit, and arrows move the IDE caret.
            self.engine
                .for_each_caret(self.doc, |engine, doc| engine.modeless_key(doc, vkey))
        } else {
            self.engine.press_key(self.doc, vkey)
        };
        self.out.effects.extend(effects);
        true
    }

    /// In Insert mode, type the text events (minus echoes of keys already
    /// handled); elsewhere discard them so stale input (the `:` and `q` of
    /// `:q`) can never leak into a later Insert session.
    fn take_typed_text(&mut self, canvas: &Canvas<'_>, mut handled_chars: Vec<char>) {
        if self.engine.mode() != Mode::Insert {
            canvas.ui.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(_)));
            });
            return;
        }
        let mut typed = String::new();
        canvas.ui.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Text(text) = event {
                    if !echo_of_handled_key(text, &mut handled_chars) {
                        typed.push_str(text);
                    }
                    false
                } else {
                    true
                }
            });
        });
        if !typed.is_empty() {
            self.out.effects.extend(
                self.engine
                    .for_each_caret(self.doc, |engine, doc| engine.type_text(doc, &typed)),
            );
        }
    }
}
