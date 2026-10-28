use super::super::super::*;
use super::Viewer;

impl Viewer<'_> {
    pub(super) fn pane_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        tab: &mut Tab,
        pane: egui_dock::NodePath,
    ) {
        if let Tab::Terminal(sid) = tab {
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            ui.separator();
        }
        self.app
            .new_terminal_menu(super::super::menus::NewTerminalMenuInput {
                ui,
                pane: Some(pane),
                strip: self.strip,
                window: self.window,
            });
        ui.separator();
        // Detach, dock-back, and float ops reshape main-dock top-level
        // tabs; a floating window owns a single dock with no tab layer,
        // so they stay hidden there.
        if !self.strip && self.window.is_none() {
            self.pane_layout_items(ui, tab, pane);
        }
        if let Tab::Terminal(sid) = tab {
            self.terminal_pane_items(ui, sid);
        }
    }

    /// Detach tears this pane off into a fresh top-level tab; dock-back
    /// returns it to a sibling tab. Both queue while the workspace is
    /// checked out and apply once it is back.
    fn pane_layout_items(&mut self, ui: &mut egui::Ui, tab: &Tab, pane: egui_dock::NodePath) {
        let stacked = self
            .app
            .pane_tabs
            .get(&pane)
            .is_some_and(|tabs| tabs.len() > 1);
        let detachable = stacked || !self.app.dock_back_targets.is_empty();
        let floatable = crate::App::floatable(tab);
        if detachable {
            self.pane_detach_items(ui, tab);
        }
        if floatable && appearance::menu_item(ui, "Float window", "ExternalLink", "").clicked() {
            self.app.float_pane = Some(tab.clone());
            ui.close();
        }
        if detachable || floatable {
            ui.separator();
        }
    }

    fn pane_detach_items(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        if appearance::menu_item(ui, "Detach to new tab", "PanelsTopLeft", "").clicked() {
            self.app.detach_pane = Some(tab.clone());
            ui.close();
        }
        let targets = self.app.dock_back_targets.clone();
        if targets.is_empty() {
            return;
        }
        let _ = appearance::text_menu_button(ui, "Move to tab", |ui| {
            for (id, label, icon) in targets {
                if appearance::menu_item(ui, &label, icon, "").clicked() {
                    self.app.dock_back_pane = Some((tab.clone(), id));
                    ui.close();
                }
            }
        });
    }

    /// Main/strip moves and scrollback actions for a terminal pane.
    fn terminal_pane_items(&mut self, ui: &mut egui::Ui, sid: &str) {
        let shell = self
            .app
            .state
            .sessions
            .iter()
            .any(|session| session.id == sid && session.kind == SessionKind::Shell);
        if self.strip
            && shell
            && appearance::menu_item(
                ui,
                "Move to main pane",
                "ArrowUp",
                &self.app.shortcut_label(Action::MoveToMain),
            )
            .clicked()
        {
            self.app.queue_strip_move(sid);
            ui.close();
        }
        // Main/strip moves have no meaning inside a floating window.
        if !self.strip
            && shell
            && self.window.is_none()
            && self.app.preferences.ide_mode
            && appearance::menu_item(
                ui,
                "Move to lower pane",
                "ArrowDown",
                &self.app.shortcut_label(Action::MoveToStrip),
            )
            .clicked()
        {
            self.app.queue_main_to_strip(sid);
            ui.close();
        }
        if appearance::menu_item(
            ui,
            "Search scrollback",
            "Search",
            &self.app.shortcut_label(Action::SearchScrollback),
        )
        .clicked()
        {
            self.app.open_scrollback_search(ui.ctx(), sid);
            ui.close();
        }
        if appearance::menu_item(
            ui,
            "Clear saved scrollback",
            "Eraser",
            &self.app.shortcut_label(Action::ClearScrollback),
        )
        .clicked()
        {
            self.app.send(Request::ClearHistory {
                session: Some(sid.to_owned()),
            });
            self.app.texts.remove(&format!("history:{sid}"));
            ui.close();
        }
    }
}
