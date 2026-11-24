//! Non-editor open workspace tabs shown in the projects sidebar (Flows,
//! Excalidraw, Browser, Tables, Images, VNC, Remote apps, and Git views).
use super::*;

/// One visible entry inside a workspace tab node in the projects sidebar.
pub(super) enum TreeItem {
    Session(String),
    Tab(Tab),
}

/// True when a workspace pane is a non-editor, non-terminal tab that belongs
/// in the projects sidebar tree.
pub(crate) fn is_sidebar_non_session_tab(tab: &Tab) -> bool {
    !matches!(
        tab,
        Tab::Terminal(_) | Tab::NativeEditor { .. } | Tab::Player
    )
}

impl App {
    /// Count of open non-editor, non-terminal tabs owned by `project_id`
    /// across its workspace and floating windows.
    pub(super) fn sidebar_non_session_tab_count(&self, project_id: &str) -> usize {
        let docked = self
            .layouts
            .get(project_id)
            .map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(|group| group.layout.iter_all_tabs())
                    .filter(|(_, tab)| is_sidebar_non_session_tab(tab))
                    .count()
            })
            .unwrap_or(0);
        let floated = self
            .floating_tabs()
            .filter(|(project, tab)| {
                project.as_str() == project_id && is_sidebar_non_session_tab(tab)
            })
            .count();
        docked.saturating_add(floated)
    }

    /// Floated non-editor, non-terminal tabs belonging to `project_id`.
    pub(super) fn floated_non_session_tabs(&self, project_id: &str) -> Vec<Tab> {
        self.floating_tabs()
            .filter(|(project, tab)| {
                project.as_str() == project_id && is_sidebar_non_session_tab(tab)
            })
            .map(|(_, tab)| tab.clone())
            .collect()
    }

    /// Renders one non-editor open tab row in the projects sidebar. Clicking
    /// activates and focuses that tab; right-click offers Close (and Rename
    /// tab when it stands alone for an unnamed workspace tab).
    pub(super) fn sidebar_tab_row(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, tab_id): (&str, Option<&str>),
        tab: &Tab,
    ) -> Option<egui::Response> {
        let height = appearance::SESSION_ROW_HEIGHT;
        if super::super::explorer_rows::skip_clipped_row(ui, height) {
            return None;
        }
        let face = self.tab_face(Some(tab));
        let selected = self.selected.as_deref() == Some(project_id)
            && self.center == crate::CenterPage::Workspace
            && self
                .layouts
                .get(project_id)
                .and_then(|workspace| workspace.active_pane())
                == Some(tab);
        let tint = appearance::color(&self.theme.secondary);
        let response = appearance::project_row_with(
            appearance::ProjectRowInput {
                ui,
                label: &face.label,
                icon: face.icon,
                color: None,
                selected,
                height,
                trailing: "",
                tint,
            },
            appearance::RowExtras::default(),
        )
        .on_hover_text(&face.label);
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("sidebar-open-tab:{}", tab.key()),
            response.rect,
        );
        if response.clicked() {
            self.focus_sidebar_tab(ui.ctx(), project_id, tab);
        }
        appearance::context_menu(&response, |ui| {
            self.sidebar_tab_menu(ui, (project_id, tab_id), tab);
        });
        Some(response)
    }

    fn focus_sidebar_tab(&mut self, ctx: &egui::Context, project_id: &str, tab: &Tab) {
        self.select_project(project_id.to_owned());
        self.hide_center_overlay();
        if let Some(workspace) = self.layouts.get_mut(project_id)
            && workspace.activate_containing(tab)
        {
            if let Some(path) = workspace.find_tab(tab) {
                workspace.set_focused_node_and_surface(path.node_path());
            }
            self.focus_non_terminal();
            return;
        }
        for window in &mut self.floating {
            let Some(dock) = window.dock.as_mut() else {
                continue;
            };
            if let Some(path) = dock.find_tab(tab) {
                let _ = dock.set_active_tab(path);
                ctx.send_viewport_cmd_to(window.viewport, egui::ViewportCommand::Focus);
                return;
            }
        }
    }

    fn sidebar_tab_menu(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, tab_id): (&str, Option<&str>),
        tab: &Tab,
    ) {
        if let Some(tab_id) = tab_id
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Rename tab…",
                icon: "Pencil",
                shortcut: "",
            })
            .clicked()
        {
            self.begin_group_rename(project_id, tab_id, String::new());
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Close tab",
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.close_sidebar_tab(project_id, tab);
            ui.close();
        }
    }

    fn close_sidebar_tab(&mut self, project_id: &str, tab: &Tab) {
        if let Some(workspace) = self.layouts.get_mut(project_id)
            && workspace.take_pane(tab).is_some()
        {
            self.pane_index = None;
            self.pending_layout_save = true;
            return;
        }
        for window in &mut self.floating {
            let Some(dock) = window.dock.as_mut() else {
                continue;
            };
            if let Some(path) = dock.find_tab(tab) {
                let _ = dock.remove_tab(path);
                self.pane_index = None;
                self.pending_layout_save = true;
                return;
            }
        }
    }

    /// Visible sessions and non-editor tabs per workspace tab, in layout order.
    pub(super) fn project_tab_items(
        &self,
        project_id: &str,
        live_ids: &std::collections::HashSet<&str>,
    ) -> (
        Vec<(String, Vec<TreeItem>)>,
        std::collections::HashSet<String>,
    ) {
        let mut placed = std::collections::HashSet::new();
        let groups = self
            .layouts
            .get(project_id)
            .map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .map(|group| {
                        let items = group
                            .layout
                            .iter_all_tabs()
                            .filter_map(|(_, tab)| match tab {
                                Tab::Terminal(sid)
                                    if live_ids.contains(sid.as_str())
                                        && placed.insert(sid.clone()) =>
                                {
                                    Some(TreeItem::Session(sid.clone()))
                                }
                                tab if is_sidebar_non_session_tab(tab) => {
                                    Some(TreeItem::Tab(tab.clone()))
                                }
                                _ => None,
                            })
                            .collect();
                        (group.id.clone(), items)
                    })
                    .collect()
            })
            .unwrap_or_default();
        (groups, placed)
    }

    /// Display label for an unnamed workspace tab group: its primary tab's
    /// title, falling back to `Tab N` only when no title is available.
    pub(super) fn unnamed_tab_group_label(
        &self,
        project_id: &str,
        tab_id: &str,
        index: usize,
    ) -> String {
        let title = self
            .layouts
            .get(project_id)
            .and_then(|workspace| workspace.tabs.iter().find(|group| group.id == tab_id))
            .and_then(|group| Self::group_primary(group))
            .map(|tab| self.tab_face(Some(tab)).label)
            .filter(|label| !label.trim().is_empty());
        title.unwrap_or_else(|| crate::workspace::group_display_name(None, index))
    }

    pub(super) fn paint_flat_tree_items(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, items): (&str, &[TreeItem]),
        by_id: &std::collections::HashMap<&str, &Arc<Session>>,
    ) {
        for item in items {
            match item {
                TreeItem::Session(sid) => {
                    if let Some(session) = by_id.get(sid.as_str()) {
                        self.session_row(ui, session);
                        self.session_children(ui, project_id, sid);
                    }
                }
                TreeItem::Tab(tab) => {
                    self.sidebar_tab_row(ui, (project_id, None), tab);
                }
            }
        }
    }

    pub(super) fn paint_overflow_items(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, overflow, floated): (&str, &[String], &[Tab]),
        by_id: &std::collections::HashMap<&str, &Arc<Session>>,
    ) {
        for sid in overflow {
            if let Some(session) = by_id.get(sid.as_str()) {
                self.session_row(ui, session);
                self.session_children(ui, project_id, sid);
            }
        }
        for tab in floated {
            self.sidebar_tab_row(ui, (project_id, None), tab);
        }
    }
}
