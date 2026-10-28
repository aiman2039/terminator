use super::super::super::*;
use super::Viewer;
use crate::native_editor::NativeEditorViewInput;
use crate::vnc_ui::VncViewInput;

impl TabViewer for Viewer<'_> {
    fn set_render_path(&mut self, path: egui_dock::NodePath) {
        self.render_path = Some(path);
    }
    fn set_render_tab(&mut self, path: egui_dock::TabPath) {
        self.render_tab = Some(path);
        self.set_render_path(path.node_path());
    }
    fn on_add(&mut self, path: egui_dock::NodePath) {
        if self.strip {
            self.app.add_strip_tab = Some((path, None));
        } else if let Some(viewport) = self.window {
            self.app.add_float_tab = Some((viewport, path, None));
        } else {
            self.app.add_tab = Some((path, None));
        }
    }
    type Tab = Tab;
    fn show_tab_bar(&self, path: egui_dock::NodePath) -> bool {
        // Stacked leaves read as nested tabs: a leaf holding more than
        // one pane shows its bar even in the main dock, whose single
        // panes keep the workspace strip as their only tab row. Floating
        // windows read their own maps; a lone floated pane keeps the OS
        // window title as its only tab row, like before.
        if self.strip {
            return true;
        }
        let tabs = if self.window.is_some() {
            &self.app.float_pane_tabs
        } else {
            &self.app.pane_tabs
        };
        tabs.get(&path).is_some_and(|tabs| {
            tabs.len() > 1
                || tabs
                    .iter()
                    .any(|tab| matches!(tab, Tab::NativeEditor { .. }))
        })
    }
    fn trailing_controls_width(&self) -> f32 {
        let actions = if self.strip {
            appearance::strip_terminal_actions_width() + 4.0
        } else {
            0.0
        };
        actions + 28.0
    }
    fn trailing_controls(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        self.pane_trailing_controls(ui, path);
    }
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        if matches!(tab, Tab::NativeEditor { .. }) {
            egui::Id::unique((tab.key(), &self.project, &self.tab, self.render_tab))
        } else {
            egui::Id::unique(tab.key())
        }
    }
    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        self.tab_title(tab)
    }
    fn tab_leading_width(&self, tab: &Tab) -> f32 {
        match tab {
            Tab::Terminal(sid) => self.app.tab_leading(sid).width(),
            _ => 0.0,
        }
    }
    fn paint_tab_leading(&mut self, ui: &mut egui::Ui, rect: egui::Rect, tab: &mut Tab) {
        self.paint_terminal_leading(ui, rect, tab);
    }
    fn allowed_in_windows(&self, _: &mut Tab) -> bool {
        false
    }
    fn scroll_bars(&self, _: &Tab) -> [bool; 2] {
        [false, false]
    }
    fn on_close(&mut self, tab: &mut Tab, pane: egui_dock::NodePath) -> OnCloseResponse {
        self.close_tab(tab, pane)
    }
    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_button(tab, response);
    }
    fn on_tab_button_at(
        &mut self,
        tab: &mut Tab,
        path: egui_dock::TabPath,
        response: &egui::Response,
    ) -> bool {
        self.tab_button_at(tab, path, response)
    }
    fn context_menu_at(&mut self, ui: &mut egui::Ui, tab: &mut Tab, path: egui_dock::TabPath) {
        self.context_menu(ui, tab, path.node_path());
        if matches!(tab, Tab::NativeEditor { .. })
            && (self.app.detach_pane.is_some()
                || self.app.float_pane.is_some()
                || self.app.dock_back_pane.is_some())
            && let (Some(project), Some(group)) = (&self.project, &self.tab)
        {
            self.app.pane_action_source = Some(PaneSource {
                project: project.clone(),
                group: group.clone(),
                path,
            });
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Tab, pane: egui_dock::NodePath) {
        self.pane_context_menu(ui, tab, pane);
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Image { path } => self.app.image_view(ui, path),
            Tab::Browser { id, target } => {
                let key = id.clone();
                self.app.browser_view(ui, key, target);
            }
            Tab::Player => {
                self.app.open_player();
                ui.close();
            }
            Tab::Diff { .. } => self.app.diff_view(ui, tab),
            Tab::NativeEditor { path } => self.app.native_editor_view(NativeEditorViewInput {
                ui,
                path,
                project: self.project.as_deref(),
                tab: self.tab.as_deref(),
                node: self.render_path,
            }),
            Tab::CommitLog { .. } => self.app.commit_log_view(ui, tab),
            Tab::Blame { .. } => self.app.blame_view(ui, tab),
            Tab::RevisionDiff { .. } => self.app.revision_diff_view(ui, tab),
            Tab::FileHistory { .. } => self.app.file_history_view(ui, tab),
            Tab::Vnc {
                id,
                session,
                profile_id,
                host,
                port,
                label,
            } => {
                let tab_id = id.clone();
                self.app.vnc_view(
                    ui,
                    VncViewInput {
                        tab_id: &tab_id,
                        session_id: session,
                        profile_id,
                        host,
                        port,
                        label,
                    },
                );
            }
            Tab::Terminal(sid) => self.terminal_pane(ui, sid),
        }
    }
}
