use super::super::super::*;
use super::Viewer;

impl Viewer<'_> {
    /// A terminal tab's body: its caption, then the dead-session view, a
    /// reconnect notice, or the live markdown or terminal view.
    pub(super) fn terminal_pane(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some(session) = self
            .app
            .state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .cloned()
        else {
            // The daemon pruned the record (a plain shell or editor
            // without an agent resume handle). Drop the stale tab
            // instead of leaving a dead placeholder behind.
            self.app.queue_unavailable_tab_close(sid);
            return;
        };
        let editing = self.app.renaming(sid, RenameSurface::Pane);
        if self.strip {
            self.strip_rename_row(ui, sid, editing);
        } else {
            self.pane_caption(ui, &session, editing);
        }
        if !session.lifecycle.live() {
            self.dead_session_view(ui, &session);
            return;
        }
        if !self.app.connected {
            ui.weak("Reconnecting to session daemon…");
            return;
        }
        self.app.draw_unsaved_close_bar(ui, sid);
        if markdown::available(&session) {
            self.markdown_view(ui, &session);
        } else {
            self.terminal_view(ui, &session);
        }
    }

    /// Native tab bars title strip panes, so there is no caption chrome
    /// (close, drag, and menus all live on the tab). Only an in-progress
    /// rename needs a row of its own.
    fn strip_rename_row(&mut self, ui: &mut egui::Ui, sid: &str, editing: bool) {
        if !editing {
            return;
        }
        let slot =
            ui.allocate_response(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
        self.app
            .inline_rename(super::super::menus::InlineRenameInput {
                ui,
                sid,
                surface: RenameSurface::Pane,
                rect: slot.rect,
            });
    }
}
