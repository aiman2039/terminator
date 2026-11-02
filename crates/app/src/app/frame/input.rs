//! Start-of-frame bookkeeping and global keyboard shortcuts.

use crate::*;

impl App {
    /// Resets the per-frame visibility sets that painting refills.
    pub(super) fn begin_frame_visibility(&mut self, ctx: &egui::Context) {
        self.hover_popup_blocks_input = self.hover_popup.is_some()
            || (ctx.current_pass_index() > 0 && self.hover_popup_blocks_input);
        // Capture modifier-only intent before global shortcuts consume keys.
        crate::workspace_ui::link_menu_modifier_pressed(ctx);
        self.visible_dirs.clear();
        self.visible_sessions.clear();
        self.visible_images.clear();
        self.visible_browsers.clear();
        for (key, view) in &mut self.vnc_views {
            view.visible_last_frame = self.visible_vnc.contains(key);
        }
        self.visible_vnc.clear();
        self.markdown.begin_frame();
        #[cfg(feature = "test-support")]
        self.diagnostics.frame(ctx);
    }

    /// F8 releases VNC shortcut capture and F11 leaves VNC fullscreen; both
    /// keys are consumed so nothing else sees them.
    pub(super) fn handle_vnc_escape_keys(&mut self, ctx: &egui::Context) {
        if self.vnc_captures_shortcuts(ctx) && ctx.input(|i| i.key_pressed(egui::Key::F8)) {
            for view in self.vnc_views.values_mut() {
                view.capture_shortcuts = false;
            }
            Self::drop_key_events(ctx, egui::Key::F8);
        }
        if self.vnc_fullscreen.is_some() && ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.exit_vnc_fullscreen(ctx);
            Self::drop_key_events(ctx, egui::Key::F11);
        }
    }

    pub(crate) fn drop_key_events(ctx: &egui::Context, key: egui::Key) {
        ctx.input_mut(|i| {
            i.events.retain(
                |event| !matches!(event, egui::Event::Key { key: pressed, .. } if *pressed == key),
            );
        });
    }

    pub(super) fn run_shortcuts(&mut self, ctx: &egui::Context) {
        let capture_vnc = self.vnc_captures_shortcuts(ctx);
        let editor_focus = terminator_native_edit::view::source_has_focus(ctx);
        let jetbrains = editor_focus
            && crate::appearance::editor_style()
                .prefs
                .editor_jetbrains_keys;
        if jetbrains && !capture_vnc {
            self.run_jetbrains_shortcuts(ctx);
        } else {
            self.double_shift = Default::default();
        }
        for &action in Action::ALL {
            if capture_vnc {
                continue;
            }
            if !self.shortcut_allowed(action) || !self.shortcut_applies(action) {
                continue;
            }
            if action == Action::SelectAll && self.text_input_focused(ctx) {
                continue;
            }
            let key = shortcuts::binding(&self.state.settings.keybindings, action.id());
            if editor_focus && shortcuts::editor_owns(&key, jetbrains) {
                continue;
            }
            if key.is_empty() || !shortcuts::consume(ctx, &key) {
                continue;
            }
            self.run_shortcut(ctx, action);
        }
    }

    /// `JetBrains` keymap app chords and double Shift, while a native
    /// editor has focus; they run before the normal bindings.
    fn run_jetbrains_shortcuts(&mut self, ctx: &egui::Context) {
        for &(chord, action) in shortcuts::jetbrains_app_chords() {
            if shortcuts::consume(ctx, chord) {
                self.run_shortcut(ctx, action);
            }
        }
        let (shift, other) = ctx.input(|input| {
            let m = input.modifiers;
            (
                m.shift && !m.command && !m.ctrl && !m.alt,
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::PointerButton { .. }
                    )
                }),
            )
        });
        if self
            .double_shift
            .update(shift, other, std::time::Instant::now())
        {
            self.run_shortcut(ctx, Action::OpenPalette);
        }
    }
}
