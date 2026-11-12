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
        self.tables.begin_frame();
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
        if self.remote_app_captures_keys(ctx) && ctx.input(|i| i.key_pressed(egui::Key::F8)) {
            self.release_remote_app_focus(ctx);
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

    pub(crate) fn run_shortcuts(&mut self, ctx: &egui::Context) {
        let jetbrains = crate::appearance::editor_style()
            .prefs
            .editor_jetbrains_keys;
        self.dispatch_shortcuts(ctx, jetbrains);
    }

    /// The Settings bindings, after the `JetBrains` keymap chords and
    /// double Shift when `jetbrains` is on. VNC shortcut capture takes
    /// every key. Esc out of a tool window comes first, before any pane
    /// can read it.
    pub(crate) fn dispatch_shortcuts(&mut self, ctx: &egui::Context, jetbrains: bool) {
        shortcuts::note_frame_focus(ctx);
        // A diff key lives through the pass it was set in and the next one
        // (a palette command runs after the diff painted); one no diff took
        // must not fire on a later hover.
        let pass = ctx.cumulative_pass_nr();
        self.diff_nav = self
            .diff_nav
            .filter(|(_, set)| set.saturating_add(1) >= pass);
        self.escape_tool_window(ctx);
        // A focused remote app window takes every key, like VNC capture;
        // F8 or a click elsewhere gives the keys back.
        let keys_captured = self.vnc_captures_shortcuts(ctx) || self.remote_app_captures_keys(ctx);
        let editor_focus = terminator_native_edit::view::source_has_focus(ctx);
        let terminal_focus = shortcuts::terminal_has_focus(ctx);
        if jetbrains && !keys_captured {
            self.run_jetbrains_shortcuts(ctx, editor_focus);
        } else {
            self.double_shift = Default::default();
        }
        // Shared chords go by the looser keyboard owner: the terminal's own
        // find bar counts as the terminal.
        let terminal_keyboard = self.terminal_has_keyboard(ctx);
        if !keys_captured {
            self.browser_keys(ctx);
        }
        for &action in Action::ALL {
            if keys_captured {
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
            if terminal_focus && shortcuts::terminal_keeps(&key) {
                continue;
            }
            // Shared chords (Cmd+Shift+R; Ctrl+Shift+F on Linux) go by focus.
            if shortcuts::yields(&self.state.settings.keybindings, action, terminal_keyboard) {
                continue;
            }
            if key.is_empty() || !shortcuts::consume(ctx, &key) {
                continue;
            }
            self.run_shortcut(ctx, action);
        }
    }

    /// A terminal has the keyboard: its pane is the focused one, and no
    /// text field, Explorer tree or tool window row took the keys from it.
    /// The terminal's own find bar (see `find_in_active_terminal`) counts
    /// as the terminal.
    pub(crate) fn terminal_has_keyboard(&self, ctx: &egui::Context) -> bool {
        let Some(sid) = self.active_session.as_deref() else {
            return false;
        };
        let find_bar = egui::Id::unique(("terminal-find", sid));
        ctx.memory(|memory| memory.has_focus(find_bar))
            || shortcuts::terminal_has_focus(ctx)
            || (!self.text_input_focused(ctx)
                && !crate::native_editor::explorer_has_focus(ctx)
                && shortcuts::focused_sidebar(ctx).is_none())
    }

    /// `JetBrains` keymap app chords and double Shift, before the Settings
    /// bindings. Each chord has its own reach: the palette chords work in
    /// every pane, Back and Forward only in a focused editor.
    fn run_jetbrains_shortcuts(&mut self, ctx: &egui::Context, editor_focus: bool) {
        let terminal_focus = shortcuts::terminal_has_focus(ctx);
        for &(chord, command, reach) in shortcuts::jetbrains_app_chords() {
            let gate = command.gate();
            if reach.applies(editor_focus, terminal_focus)
                && self.shortcut_allowed(gate)
                && self.shortcut_applies(gate)
                && shortcuts::consume(ctx, chord)
            {
                self.run_jetbrains_command(ctx, command);
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
            && self.shortcut_allowed(Action::OpenPalette)
        {
            self.run_shortcut(ctx, Action::OpenPalette);
        }
    }

    /// Find Action and Go to File open the palette with one filter on.
    fn run_jetbrains_command(&mut self, ctx: &egui::Context, command: shortcuts::JetBrainsCommand) {
        let filter = match command {
            shortcuts::JetBrainsCommand::Run(action) => {
                self.run_shortcut(ctx, action);
                return;
            }
            shortcuts::JetBrainsCommand::FindAction => crate::palette::FILTER_ACTIONS,
            shortcuts::JetBrainsCommand::GotoFile => crate::palette::FILTER_FILES,
        };
        self.open_command_palette();
        self.palette.filters = filter;
    }
}
