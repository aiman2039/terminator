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
            self.vnc_fullscreen = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            Self::drop_key_events(ctx, egui::Key::F11);
        }
    }

    fn drop_key_events(ctx: &egui::Context, key: egui::Key) {
        ctx.input_mut(|i| {
            i.events.retain(
                |event| !matches!(event, egui::Event::Key { key: pressed, .. } if *pressed == key),
            );
        });
    }

    pub(super) fn run_shortcuts(&mut self, ctx: &egui::Context) {
        let capture_vnc = self.vnc_captures_shortcuts(ctx);
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
            if terminator_native_edit::view::source_has_focus(ctx) && shortcuts::editor_owns(&key) {
                continue;
            }
            if key.is_empty() || !shortcuts::consume(ctx, &key) {
                continue;
            }
            self.run_shortcut(ctx, action);
        }
    }
}
