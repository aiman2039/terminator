use super::super::super::*;
use super::Viewer;

impl Viewer<'_> {
    pub(super) fn pane_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        tab: &mut Tab,
        (pane, at): (egui_dock::NodePath, Option<egui_dock::TabPath>),
    ) {
        if let Tab::Terminal(sid) = tab {
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            ui.separator();
        }
        if let Tab::NativeEditor { path } = tab {
            self.app.native_file_menu(ui, path);
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
            self.zoom_menu_item(ui, tab, pane);
            self.pane_layout_items(ui, tab, (pane, at));
        } else if let Some(viewport) = self.window {
            self.float_dock_back_items(ui, tab, viewport);
        }
        if let Tab::Terminal(sid) = tab {
            self.terminal_pane_items(ui, sid);
        }
    }

    /// Detach tears this pane off into a fresh top-level tab; dock-back
    /// returns it to a sibling tab. Both queue while the workspace is
    /// checked out and apply once it is back.
    fn pane_layout_items(
        &mut self,
        ui: &mut egui::Ui,
        tab: &Tab,
        (pane, at): (egui_dock::NodePath, Option<egui_dock::TabPath>),
    ) {
        let source = self.view_source(tab, at);
        let stacked = self
            .app
            .pane_tabs
            .get(&pane)
            .is_some_and(|tabs| tabs.len() > 1);
        let detachable = stacked || !self.app.dock_back_targets.is_empty();
        let floatable = crate::App::floatable(tab);
        if detachable {
            self.pane_detach_items(ui, tab, source.clone());
        }
        if floatable
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Float window",
                icon: "ExternalLink",
                shortcut: "",
            })
            .clicked()
        {
            self.app.queue_dock_op(DockOp::Float {
                pane: tab.clone(),
                source,
            });
            ui.close();
        }
        if detachable || floatable {
            ui.separator();
        }
    }

    /// A floating window's way home: this pane to its old place, or the
    /// whole window (the same as closing it).
    fn float_dock_back_items(&mut self, ui: &mut egui::Ui, tab: &Tab, viewport: egui::ViewportId) {
        let back = "Dock back to main window";
        if appearance::menu_action(ui, true, back, "ArrowDownToLine").clicked() {
            self.app.queue_dock_op(DockOp::FloatReturn {
                viewport,
                pane: Some(tab.clone()),
            });
        }
        // The pane maps hold only the window under render.
        let panes = self
            .app
            .float_pane_tabs
            .values()
            .map(Vec::len)
            .sum::<usize>();
        if panes > 1
            && appearance::menu_action(ui, true, "Dock back all", "PanelsTopLeft").clicked()
        {
            self.app.queue_dock_op(DockOp::FloatReturn {
                viewport,
                pane: None,
            });
        }
        ui.separator();
    }

    /// The exact native editor view a pane menu acts on. Same-file views
    /// share a tab identity, so moves need the view's address; other panes
    /// move by identity.
    fn view_source(&self, tab: &Tab, at: Option<egui_dock::TabPath>) -> Option<PaneSource> {
        let (Some(project), Some(group), Some(path)) = (&self.project, &self.tab, at) else {
            return None;
        };
        matches!(tab, Tab::NativeEditor { .. }).then(|| PaneSource {
            project: project.clone(),
            group: group.clone(),
            path,
        })
    }

    fn pane_detach_items(&mut self, ui: &mut egui::Ui, tab: &Tab, source: Option<PaneSource>) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Detach to new tab",
            icon: "PanelsTopLeft",
            shortcut: "",
        })
        .clicked()
        {
            self.app.queue_dock_op(DockOp::Detach {
                pane: tab.clone(),
                source: source.clone(),
            });
            ui.close();
        }
        let targets = self.app.dock_back_targets.clone();
        if targets.is_empty() {
            return;
        }
        let _ = appearance::text_menu_button(ui, "Move to tab", |ui| {
            for (id, label, icon) in targets {
                if appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label: &label,
                    icon,
                    shortcut: "",
                })
                .clicked()
                {
                    self.app.queue_dock_op(DockOp::DockBack {
                        pane: tab.clone(),
                        dest: id,
                        source: source.clone(),
                    });
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
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Move to main pane",
                icon: Action::MoveToMain.icon(),
                shortcut: &self.app.shortcut_label(Action::MoveToMain),
            })
            .clicked()
        {
            self.app.queue_strip_move(sid);
            ui.close();
        }
        // The docks are checked out while they paint, so the location test
        // stays local; `App::session_action_applies` is the same rule for
        // shortcuts and the palette.
        if !self.strip
            && shell
            && self.window.is_none()
            && self.app.preferences.ide_mode
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Move to lower pane",
                icon: Action::MoveToStrip.icon(),
                shortcut: &self.app.shortcut_label(Action::MoveToStrip),
            })
            .clicked()
        {
            self.app.queue_main_to_strip(sid);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: Action::SearchScrollback.label(),
            icon: Action::SearchScrollback.icon(),
            shortcut: &self.app.shortcut_label(Action::SearchScrollback),
        })
        .clicked()
        {
            self.app.open_scrollback_search(ui.ctx(), sid);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: Action::ClearScrollback.label(),
            icon: Action::ClearScrollback.icon(),
            shortcut: &self.app.shortcut_label(Action::ClearScrollback),
        })
        .clicked()
        {
            self.app.send(Request::ClearHistory {
                session: Some(sid.to_owned()),
            });
            self.app.texts.remove(&format!("history:{sid}"));
            ui.close();
        }
        self.app.composer_menu_item(ui, sid);
        self.app.last_turn_menu_item(ui, sid);
    }
}
