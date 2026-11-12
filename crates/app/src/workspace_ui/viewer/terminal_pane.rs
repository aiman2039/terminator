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
        if self.strip || self.tab_row_titles_pane(&session) {
            self.strip_rename_row(ui, sid, editing);
        } else {
            self.pane_caption(ui, &session, editing);
        }
        if !session.lifecycle.live() {
            self.dead_session_view(ui, &session);
            return;
        }
        let tab = Tab::Terminal(session.id.clone());
        if self.zoomed_here(&tab) {
            Self::zoom_placeholder(ui);
            return;
        }
        self.live_terminal_body(ui, &tab, &session);
    }

    /// The live terminal (or its markdown view) under the caption; the
    /// zoom overlay paints the same body.
    pub(super) fn terminal_body(&mut self, ui: &mut egui::Ui, session: &Session) {
        self.app.draw_unsaved_close_bar(ui, &session.id);
        if markdown::available(session) {
            self.markdown_view(ui, session);
        } else {
            self.terminal_view(ui, session);
        }
    }

    /// A main-dock leaf that shows its own tab bar (stacked tabs) already
    /// gives the pane its title, close, New tab and split menu, so the
    /// caption would repeat that row. The caption stays where it carries
    /// something the tab bar has not: markdown mode buttons and the Resume
    /// chip.
    fn tab_row_titles_pane(&self, session: &Session) -> bool {
        use egui_dock::TabViewer as _;
        self.window.is_none()
            && self.render_path.is_some_and(|path| self.show_tab_bar(path))
            && !(session.remote.is_none() && markdown::available(session))
            && self.app.resume_chip_command(&session.id).is_none()
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
        // The field itself is frameless, so this row would read as terminal
        // text. Draw an input box (surface fill, focus stroke) behind it.
        let field = slot.rect.shrink2(egui::vec2(6.0, 2.0));
        let visuals = ui.visuals();
        ui.painter().rect(
            field,
            appearance::RADIUS_SMALL,
            visuals.extreme_bg_color,
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
        self.app
            .inline_rename(super::super::menus::InlineRenameInput {
                ui,
                sid,
                surface: RenameSurface::Pane,
                rect: field.shrink2(egui::vec2(6.0, 0.0)),
            });
    }
}
