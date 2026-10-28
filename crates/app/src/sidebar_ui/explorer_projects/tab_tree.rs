//! Live sessions under a project, grouped per workspace tab.
use super::*;

impl App {
    /// Live terminal sessions under one project, grouped as a tree per
    /// top-level workspace tab in layout order. Single-tab projects keep the
    /// historic flat list. Multi-tab projects render one expandable tab node
    /// per tab that owns live sessions, then any background sessions that are
    /// in no tab. A tab that holds exactly one live terminal and has no custom
    /// name draws that terminal in the tab's place, because the node would
    /// only repeat it.
    pub(super) fn project_sessions_tree(&mut self, ui: &mut egui::Ui, project_id: &str) {
        let index = self.sidebar_index();
        let live: Vec<Arc<Session>> = index
            .by_project
            .get(project_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| {
                        s.lifecycle.live()
                            && s.kind != SessionKind::Editor
                            && crate::daemon_connection::session_owner_available(&self.state, s)
                    })
                    .map(Arc::clone)
                    .collect()
            })
            .unwrap_or_default();
        if live.is_empty() {
            return;
        }
        let tabs: Vec<(String, Vec<String>)> = self
            .layouts
            .get(project_id)
            .map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .map(|group| {
                        (
                            group.id.clone(),
                            group
                                .layout
                                .iter_all_tabs()
                                .filter_map(|(_, tab)| match tab {
                                    Tab::Terminal(sid) => Some(sid.clone()),
                                    _ => None,
                                })
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if tabs.len() < 2 {
            for session in &live {
                self.session_row(ui, session);
            }
            return;
        }
        let live_ids: Vec<String> = live.iter().map(|s| s.id.clone()).collect();
        let (groups, overflow) = group_session_ids_by_tab(&tabs, &live_ids);
        let by_id: std::collections::HashMap<&str, &Arc<Session>> =
            live.iter().map(|s| (s.id.as_str(), s)).collect();
        let workspace_active = self.layouts.get(project_id).map(|w| w.active.clone());
        for (tab_id, sids) in &groups {
            // The group is abstract: its own name, or `Tab N` by position.
            // It never borrows a member terminal's label.
            let (group_index, group_name) = self
                .layouts
                .get(project_id)
                .and_then(|workspace| {
                    workspace
                        .tabs
                        .iter()
                        .position(|group| &group.id == tab_id)
                        .map(|index| {
                            (
                                index,
                                workspace.tabs.get(index).and_then(|group| {
                                    group.name.clone().filter(|name| !name.trim().is_empty())
                                }),
                            )
                        })
                })
                .unwrap_or((0, None));
            let renaming_group = self
                .rename_group
                .as_ref()
                .is_some_and(|(p, t)| p == project_id && t == tab_id);
            // One live terminal and no custom name: the node would only repeat
            // the terminal it holds, so the terminal itself takes its place.
            // A tab that is being named keeps its node to host the field.
            if sids.len() == 1 && group_name.is_none() && !renaming_group {
                if let Some(session) = sids
                    .first()
                    .and_then(|sid| by_id.get(sid.as_str()))
                    .map(|session| session.as_ref())
                {
                    self.session_row_in_tab(SessionRowInTabInput {
                        ui,
                        session,
                        project_id,
                        tab_id,
                    });
                }
                continue;
            }
            let group_label =
                crate::workspace::group_display_name(group_name.as_deref(), group_index);
            let key = format!("workspace-tab:{project_id}:{tab_id}");
            let mut tab_expanded = *self.preferences.expanded.entry(key.clone()).or_insert(true);
            let active_tab = workspace_active.as_deref() == Some(tab_id.as_str());
            let selected = self.selected.as_deref() == Some(project_id) && active_tab;
            let mut rename_action = GroupRenameAction::None;
            let tab_left = ui
                .horizontal(|ui| {
                    if ui
                        .add_sized(
                            [16.0, self.theme.row_height()],
                            egui::Button::image(
                                egui::Image::new(icons::source(if tab_expanded {
                                    "ChevronDown"
                                } else {
                                    "ChevronRight"
                                }))
                                .tint(appearance::icon_color())
                                .fit_to_exact_size(egui::vec2(12.0, 12.0)),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Expand or collapse tab")
                        .clicked()
                    {
                        self.finish_rename(true);
                        tab_expanded = !tab_expanded;
                        self.preferences.expanded.insert(key.clone(), tab_expanded);
                    }
                    if renaming_group {
                        let field = ui.add_sized(
                            [160.0, self.theme.row_height()],
                            appearance::singleline(&mut self.rename_group_text)
                                .id(ui.scope_id().with(("rename-group", tab_id.as_str())))
                                .hint_text(&group_label),
                        );
                        if self.rename_group_focus {
                            field.request_focus();
                            self.rename_group_focus = false;
                        }
                        #[cfg(feature = "test-support")]
                        diagnostics::record(
                            ui.ctx(),
                            &format!("sidebar-tab-rename:{tab_id}"),
                            field.rect,
                        );
                        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                            rename_action = GroupRenameAction::Cancel;
                        } else if field.lost_focus()
                            || (field.has_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        {
                            rename_action = GroupRenameAction::Commit;
                        }
                        field.rect.left()
                    } else {
                        let response = appearance::project_row(appearance::ProjectRowInput {
                            ui,
                            label: &group_label,
                            icon: "LayoutDashboard",
                            selected,
                            height: self.theme.row_height(),
                            trailing: &sids.len().to_string(),
                            tint: appearance::color(&self.theme.secondary),
                        })
                        .on_hover_text(format!("Tab group\n{group_label}\nDouble-click to rename"));
                        #[cfg(feature = "test-support")]
                        diagnostics::record(
                            ui.ctx(),
                            &format!("sidebar-tab:{tab_id}"),
                            response.rect,
                        );
                        appearance::context_menu(&response, |ui| {
                            if appearance::menu_item(ui, "Rename tab…", "Pencil", "").clicked() {
                                rename_action = GroupRenameAction::Begin;
                                ui.close();
                            }
                            if group_name.is_some()
                                && appearance::menu_item(ui, "Clear tab name", "X", "").clicked()
                            {
                                rename_action = GroupRenameAction::Clear;
                                ui.close();
                            }
                        });
                        if response.double_clicked() {
                            rename_action = GroupRenameAction::Begin;
                        } else if response.clicked() {
                            self.select_project(project_id.to_string());
                            if let Some(workspace) = self.layouts.get_mut(project_id) {
                                workspace.active.clone_from(tab_id);
                            }
                            self.hide_center_overlay();
                        }
                        response.rect.left()
                    }
                })
                .inner;
            match rename_action {
                GroupRenameAction::Begin => {
                    self.rename_group = Some((project_id.to_string(), tab_id.clone()));
                    self.rename_group_text = group_name.clone().unwrap_or_default();
                    self.rename_group_focus = true;
                }
                GroupRenameAction::Commit => {
                    let name = self.rename_group_text.trim().to_string();
                    if let Some(group) = self
                        .layouts
                        .get_mut(project_id)
                        .and_then(|w| w.tabs.iter_mut().find(|g| &g.id == tab_id))
                    {
                        group.name = if name.is_empty() {
                            None
                        } else {
                            Some(name.chars().take(64).collect())
                        };
                    }
                    self.rename_group = None;
                    self.save_layouts();
                }
                GroupRenameAction::Cancel => {
                    self.rename_group = None;
                }
                GroupRenameAction::Clear => {
                    if let Some(group) = self
                        .layouts
                        .get_mut(project_id)
                        .and_then(|w| w.tabs.iter_mut().find(|g| &g.id == tab_id))
                    {
                        group.name = None;
                    }
                    self.save_layouts();
                }
                GroupRenameAction::None => {}
            }
            if tab_expanded {
                ui.scope(|ui| {
                    ui.spacing_mut().indent += tab_left - ui.next_widget_position().x;
                    ui.indent(format!("sidebar-tab-body:{tab_id}"), |ui| {
                        for sid in sids {
                            if let Some(session) = by_id.get(sid.as_str()) {
                                self.session_row(ui, session);
                            }
                        }
                    });
                });
            }
        }
        for sid in &overflow {
            if let Some(session) = by_id.get(sid.as_str()) {
                self.session_row(ui, session);
            }
        }
    }
}
