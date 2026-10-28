//! Live sessions under a project, grouped per workspace tab.
use super::*;

/// One workspace tab's node in the project's session tree.
struct TabGroupInput<'a> {
    project_id: &'a str,
    tab_id: &'a String,
    sids: &'a [String],
    by_id: &'a std::collections::HashMap<&'a str, &'a Arc<Session>>,
    workspace_active: Option<&'a str>,
}

/// What a tab node's header shows.
struct TabGroupHeader<'a> {
    project_id: &'a str,
    tab_id: &'a String,
    key: &'a str,
    label: &'a str,
    named: bool,
    renaming: bool,
    selected: bool,
    count: usize,
}

impl App {
    /// Live terminal sessions under one project, grouped as a tree per
    /// top-level workspace tab in layout order. Single-tab projects keep the
    /// historic flat list. Multi-tab projects render one expandable tab node
    /// per tab that owns live sessions, then any background sessions that are
    /// in no tab. A tab that holds exactly one live terminal and has no custom
    /// name draws that terminal in the tab's place, because the node would
    /// only repeat it.
    pub(super) fn project_sessions_tree(&mut self, ui: &mut egui::Ui, project_id: &str) {
        let live = self.live_project_sessions(project_id);
        if live.is_empty() {
            return;
        }
        let tabs = self.project_tab_terminals(project_id);
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
            self.session_tab_group(
                ui,
                TabGroupInput {
                    project_id,
                    tab_id,
                    sids,
                    by_id: &by_id,
                    workspace_active: workspace_active.as_deref(),
                },
            );
        }
        for sid in &overflow {
            if let Some(session) = by_id.get(sid.as_str()) {
                self.session_row(ui, session);
            }
        }
    }

    /// Live, non-editor sessions of the project whose owner is reachable.
    fn live_project_sessions(&self, project_id: &str) -> Vec<Arc<Session>> {
        let index = self.sidebar_index();
        index
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
            .unwrap_or_default()
    }

    /// Terminal session ids per workspace tab, in layout order.
    fn project_tab_terminals(&self, project_id: &str) -> Vec<(String, Vec<String>)> {
        self.layouts
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
            .unwrap_or_default()
    }

    /// Position and custom name of a workspace tab. The group is
    /// abstract: its own name, or `Tab N` by position. It never borrows a
    /// member terminal's label.
    fn tab_group_name(&self, project_id: &str, tab_id: &str) -> (usize, Option<String>) {
        self.layouts
            .get(project_id)
            .and_then(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .position(|group| group.id == tab_id)
                    .map(|index| {
                        (
                            index,
                            workspace.tabs.get(index).and_then(|group| {
                                group.name.clone().filter(|name| !name.trim().is_empty())
                            }),
                        )
                    })
            })
            .unwrap_or((0, None))
    }

    fn session_tab_group(&mut self, ui: &mut egui::Ui, input: TabGroupInput<'_>) {
        let TabGroupInput {
            project_id,
            tab_id,
            sids,
            by_id,
            workspace_active,
        } = input;
        let (group_index, group_name) = self.tab_group_name(project_id, tab_id);
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
            return;
        }
        let group_label = crate::workspace::group_display_name(group_name.as_deref(), group_index);
        let key = format!("workspace-tab:{project_id}:{tab_id}");
        let mut tab_expanded = *self.preferences.expanded.entry(key.clone()).or_insert(true);
        let active_tab = workspace_active == Some(tab_id.as_str());
        let header = TabGroupHeader {
            project_id,
            tab_id,
            key: &key,
            label: &group_label,
            named: group_name.is_some(),
            renaming: renaming_group,
            selected: self.selected.as_deref() == Some(project_id) && active_tab,
            count: sids.len(),
        };
        let (tab_left, rename_action) = ui
            .horizontal(|ui| self.tab_group_header(ui, &header, &mut tab_expanded))
            .inner;
        self.apply_group_rename((project_id, tab_id), group_name, rename_action);
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

    /// Chevron plus the tab row (or its rename field). Returns the row's
    /// left edge and the rename step the user asked for.
    fn tab_group_header(
        &mut self,
        ui: &mut egui::Ui,
        header: &TabGroupHeader<'_>,
        tab_expanded: &mut bool,
    ) -> (f32, GroupRenameAction) {
        if self.sidebar_chevron(ui, *tab_expanded, "Expand or collapse tab") {
            self.finish_rename(true);
            *tab_expanded = !*tab_expanded;
            self.preferences
                .expanded
                .insert(header.key.to_owned(), *tab_expanded);
        }
        if header.renaming {
            self.tab_group_rename_field(ui, header)
        } else {
            self.tab_group_row(ui, header)
        }
    }

    fn tab_group_rename_field(
        &mut self,
        ui: &mut egui::Ui,
        header: &TabGroupHeader<'_>,
    ) -> (f32, GroupRenameAction) {
        let field = ui.add_sized(
            [160.0, self.theme.row_height()],
            appearance::singleline(&mut self.rename_group_text)
                .id(ui.scope_id().with(("rename-group", header.tab_id.as_str())))
                .hint_text(header.label),
        );
        if self.rename_group_focus {
            field.request_focus();
            self.rename_group_focus = false;
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("sidebar-tab-rename:{}", header.tab_id),
            field.rect,
        );
        let mut rename_action = GroupRenameAction::None;
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            rename_action = GroupRenameAction::Cancel;
        } else if field.lost_focus()
            || (field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
        {
            rename_action = GroupRenameAction::Commit;
        }
        (field.rect.left(), rename_action)
    }

    fn tab_group_row(
        &mut self,
        ui: &mut egui::Ui,
        header: &TabGroupHeader<'_>,
    ) -> (f32, GroupRenameAction) {
        let label = header.label;
        let response = appearance::project_row(appearance::ProjectRowInput {
            ui,
            label,
            icon: "LayoutDashboard",
            selected: header.selected,
            height: self.theme.row_height(),
            trailing: &header.count.to_string(),
            tint: appearance::color(&self.theme.secondary),
        })
        .on_hover_text(format!("Tab group\n{label}\nDouble-click to rename"));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("sidebar-tab:{}", header.tab_id),
            response.rect,
        );
        let mut rename_action = GroupRenameAction::None;
        appearance::context_menu(&response, |ui| {
            rename_action = Self::tab_group_menu(ui, header.named);
        });
        if response.double_clicked() {
            rename_action = GroupRenameAction::Begin;
        } else if response.clicked() {
            self.select_project(header.project_id.to_string());
            if let Some(workspace) = self.layouts.get_mut(header.project_id) {
                workspace.active.clone_from(header.tab_id);
            }
            self.hide_center_overlay();
        }
        (response.rect.left(), rename_action)
    }

    fn tab_group_menu(ui: &mut egui::Ui, named: bool) -> GroupRenameAction {
        let mut action = GroupRenameAction::None;
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Rename tab…",
            icon: "Pencil",
            shortcut: "",
        })
        .clicked()
        {
            action = GroupRenameAction::Begin;
            ui.close();
        }
        if named
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Clear tab name",
                icon: "X",
                shortcut: "",
            })
            .clicked()
        {
            action = GroupRenameAction::Clear;
            ui.close();
        }
        action
    }

    /// Act on a rename step for the tab `(project_id, tab_id)`.
    fn apply_group_rename(
        &mut self,
        (project_id, tab_id): (&str, &str),
        group_name: Option<String>,
        action: GroupRenameAction,
    ) {
        match action {
            GroupRenameAction::Begin => {
                self.rename_group = Some((project_id.to_string(), tab_id.to_owned()));
                self.rename_group_text = group_name.unwrap_or_default();
                self.rename_group_focus = true;
            }
            GroupRenameAction::Commit => {
                let name = self.rename_group_text.trim().to_string();
                self.set_tab_group_name(
                    project_id,
                    tab_id,
                    (!name.is_empty()).then(|| name.chars().take(64).collect()),
                );
                self.rename_group = None;
                self.save_layouts();
            }
            GroupRenameAction::Cancel => {
                self.rename_group = None;
            }
            GroupRenameAction::Clear => {
                self.set_tab_group_name(project_id, tab_id, None);
                self.save_layouts();
            }
            GroupRenameAction::None => {}
        }
    }

    fn set_tab_group_name(&mut self, project_id: &str, tab_id: &str, name: Option<String>) {
        if let Some(group) = self
            .layouts
            .get_mut(project_id)
            .and_then(|w| w.tabs.iter_mut().find(|g| g.id == tab_id))
        {
            group.name = name;
        }
    }
}
