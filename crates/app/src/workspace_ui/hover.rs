use super::super::*;
impl App {
    pub(crate) fn dismiss_hover_popup(&mut self, ctx: &egui::Context) {
        if let Some(popup) = self.hover_popup.take() {
            #[cfg(feature = "test-support")]
            eprintln!("Terminal file menu dismissed: {}", popup.target.display());
            self.dismissed_hover = Some((popup.session, popup.key));
        }
        self.hover = None;
        ctx.request_repaint();
    }

    pub(crate) fn hover_target_dismissed(&mut self, sid: &str, key: &str) -> bool {
        if let Some((session, dismissed)) = &self.dismissed_hover
            && session == sid
        {
            if dismissed == key {
                return true;
            }
            self.dismissed_hover = None;
        }
        false
    }

    pub(crate) fn terminal_input_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && self.hover_popup.is_none()
            && !self.hover_popup_blocks_input
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
        !self.picker_active
            && self.hover_popup.is_none()
            && !self.hover_popup_blocks_input
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && self.worktree_remove.is_none()
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
