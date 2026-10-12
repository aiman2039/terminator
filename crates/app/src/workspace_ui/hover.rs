use super::super::*;

#[derive(Clone, Copy)]
struct MenuModifier {
    frame: u64,
    held: bool,
    pressed: bool,
}

/// A fresh modifier press opens the menu. Holding a modifier while moving onto
/// a link, or using a keyboard chord, does not open it.
pub(super) fn advance_menu_modifier(held: bool, events: &[egui::Event]) -> (bool, bool) {
    let mut held = held;
    let mut pressed = false;
    let chord = events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { pressed: true, .. }
                | egui::Event::Text(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Paste(_)
        )
    });
    for event in events {
        if matches!(event, egui::Event::WindowFocused(false)) {
            held = false;
            pressed = false;
        }
        if let egui::Event::ModifiersChanged(modifiers) = event {
            let next = modifiers.ctrl || modifiers.mac_cmd || modifiers.command;
            pressed |= next && !held;
            held = next;
        }
    }
    (held, pressed && !chord)
}

pub(crate) fn link_menu_modifier_pressed(ctx: &egui::Context) -> bool {
    let key = egui::Id::new(("terminal-link-menu-modifier", ctx.viewport_id()));
    let frame = ctx.cumulative_frame_nr();
    let previous = ctx.data(|data| data.get_temp::<MenuModifier>(key));
    if let Some(previous) = previous.filter(|previous| previous.frame == frame) {
        return previous.pressed;
    }
    let (held, pressed) = ctx.input(|input| {
        advance_menu_modifier(
            previous.is_some_and(|previous| previous.held),
            &input.events,
        )
    });
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            MenuModifier {
                frame,
                held,
                pressed,
            },
        )
    });
    pressed
}

#[derive(Clone)]
struct HoverCopy {
    session: String,
    target: services::Target,
    rects: Vec<egui::Rect>,
    layer: egui::LayerId,
    strip: bool,
    frame: u64,
}

impl App {
    pub(crate) fn remember_hover_copy(
        &self,
        ui: &egui::Ui,
        sid: &str,
        target: &services::Target,
        rects: Vec<egui::Rect>,
        strip: bool,
    ) {
        let key = egui::Id::new(("terminal-hover-copy", ui.ctx().viewport_id()));
        let hover = HoverCopy {
            session: sid.to_owned(),
            target: target.clone(),
            rects,
            layer: ui.layer_id(),
            strip,
            frame: ui.ctx().cumulative_frame_nr(),
        };
        ui.ctx().data_mut(|data| data.insert_temp(key, hover));
    }

    /// Run before any terminal forwards keys, including an unfocused hovered
    /// pane. Use the path that was underlined in the last visible frame.
    pub(crate) fn copy_hovered_terminal_path(&self, ctx: &egui::Context) {
        if !ctx.input(|input| clipboard::terminal_copy_requested(&input.events, input.modifiers))
            || self.text_input_focused(ctx)
            || self.active_session.as_ref().is_some_and(|sid| {
                self.backends
                    .get(sid)
                    .is_some_and(|backend| !backend.last_content().selected_text.is_empty())
            })
        {
            return;
        }
        let key = egui::Id::new(("terminal-hover-copy", ctx.viewport_id()));
        let Some(hover) = ctx.data(|data| data.get_temp::<HoverCopy>(key)) else {
            return;
        };
        let enabled = if hover.strip {
            self.strip_terminal_copy_enabled(&hover.session)
        } else if ctx.viewport_id() != egui::ViewportId::ROOT {
            self.float_terminal_copy_enabled(&hover.session)
        } else {
            self.terminal_copy_enabled(&hover.session)
        };
        if !enabled {
            return;
        }
        let target = if let Some(popup) = &self.hover_popup {
            if popup.session != hover.session {
                return;
            }
            &popup.target
        } else {
            let frame = ctx.cumulative_frame_nr();
            let fresh = hover.frame == frame || hover.frame.checked_add(1) == Some(frame);
            if !fresh
                || !ctx.pointer_hover_pos().is_some_and(|pos| {
                    hover.rects.iter().any(|rect| rect.contains(pos))
                        && ctx.layer_id_at(pos) == Some(hover.layer)
                })
            {
                return;
            }
            &hover.target
        };
        if let Some(text) = ctx.input_mut(|input| {
            clipboard::take_terminal_copy(&mut input.events, input.modifiers, &target.display())
        }) {
            ctx.copy_text(text);
        }
    }

    pub(crate) fn dismiss_hover_popup(&mut self, ctx: &egui::Context) {
        if let Some(_popup) = self.hover_popup.take() {
            #[cfg(feature = "test-support")]
            eprintln!("Terminal file menu dismissed: {}", _popup.target.display());
        }
        ctx.request_repaint();
    }

    pub(crate) fn terminal_input_enabled(&self, sid: &str) -> bool {
        self.terminal_copy_enabled(sid)
            && self.hover_popup.is_none()
            && !self.hover_popup_blocks_input
    }

    pub(crate) fn terminal_copy_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.settings_open
            && !self.player_open
            && !self.command_dialog_open()
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && !self.search_open
    }

    /// Terminals docked in the IDE strip stay interactive while a center-only
    /// view (Settings, Player, palette, or search) covers the main workspace.
    /// True modals still suspend them.
    pub(crate) fn strip_terminal_input_enabled(&self, sid: &str) -> bool {
        self.strip_terminal_copy_enabled(sid)
            && self.hover_popup.is_none()
            && !self.hover_popup_blocks_input
    }

    pub(crate) fn strip_terminal_copy_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && self.worktree_remove.is_none()
    }

    /// Terminals in floating windows live outside the main window, so the
    /// same center-only views never cover them. Same gate shape as the
    /// strip: true modals still suspend them.
    pub(crate) fn float_terminal_input_enabled(&self, sid: &str) -> bool {
        self.float_terminal_copy_enabled(sid)
            && self.hover_popup.is_none()
            && !self.hover_popup_blocks_input
    }

    pub(crate) fn float_terminal_copy_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && self.worktree_remove.is_none()
    }

    /// Copy is local to the selected terminal. A file menu may block PTY input
    /// while the terminal's selection remains available to the clipboard.
    pub(crate) fn copy_terminal_selection(&self, ctx: &egui::Context, sid: &str) {
        if self.active_session.as_deref() != Some(sid) || self.text_input_focused(ctx) {
            return;
        }
        let Some(backend) = self.backends.get(sid) else {
            return;
        };
        let selection = &backend.last_content().selected_text;
        if let Some(text) = ctx.input_mut(|input| {
            clipboard::take_terminal_copy(&mut input.events, input.modifiers, selection)
        }) {
            ctx.copy_text(text);
        }
    }

    /// A pointer press on a terminal. Drops the Explorer name field's keyboard
    /// focus without saving or cancelling that prompt.
    pub(crate) fn terminal_pressed(&mut self, ctx: &egui::Context, sid: &str) {
        self.name_prompt_focus = false;
        ctx.memory_mut(|memory| memory.surrender_focus(Self::explorer_name_prompt_id()));
        if let Some(preview) = self.markdown.entries.get_mut(sid) {
            preview.editor_focused = true;
        }
        self.active_session = Some(sid.to_owned());
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
    }

    pub(super) fn terminal_status_color(&self, session: &Session) -> Option<egui::Color32> {
        let presented = self.present_session(&session.id);
        let theme = &self.theme;
        if let Some(state) = presented.lifecycle {
            return Some(appearance::color(match state {
                AgentState::Running => &theme.status_running,
                AgentState::WaitingInput | AgentState::WaitingPermission => &theme.status_waiting,
                AgentState::Failed => &theme.status_failed,
                AgentState::Completed => &theme.accent,
                _ => &theme.secondary,
            }));
        }
        session
            .lifecycle
            .live()
            .then(|| appearance::color(&theme.status_running))
    }

    pub(crate) fn branch_at(&self, cwd: &std::path::Path) -> Option<String> {
        if let Some(metadata) = &self.metadata
            && metadata.cwd == cwd
        {
            return metadata.branch.clone().filter(|branch| !branch.is_empty());
        }
        self.context.as_ref().and_then(|context| {
            (context.cwd == cwd && !context.branch.is_empty()).then(|| context.branch.clone())
        })
    }
}
