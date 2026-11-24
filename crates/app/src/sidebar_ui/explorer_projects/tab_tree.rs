//! Live sessions and open non-editor tabs under a project, grouped per
//! workspace tab.
use super::tab_row::TreeItem;
use super::*;

/// One workspace tab's node in the project's session tree.
struct TabGroupInput<'a> {
    project_id: &'a str,
    tab_id: &'a String,
    items: &'a [TreeItem],
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
    /// Agent states of the group's sessions, or their count.
    trailing: String,
    /// The group's live agents by state: the row's state counters.
    counts: crate::agent_presence::StateCounts,
}

/// Preferences key of a workspace tab's sidebar node; present (false)
/// only while the node is collapsed.
fn tab_expanded_key(project_id: &str, tab_id: &str) -> String {
    format!("workspace-tab:{project_id}:{tab_id}")
}

impl App {
    /// Drops a closed tab's collapsed state from the preferences.
    pub(crate) fn forget_tab_expanded(&mut self, project_id: &str, tab_id: &str) {
        self.preferences
            .expanded
            .remove(&tab_expanded_key(project_id, tab_id));
    }

    /// Drops collapsed state of tabs that no longer exist in a loaded
    /// project, whatever path closed them (#209). Projects without a loaded
    /// layout keep their keys.
    pub(crate) fn prune_closed_tab_expanded(&mut self) {
        let layouts = &self.layouts;
        self.preferences.expanded.retain(|key, _| {
            let Some(rest) = key.strip_prefix("workspace-tab:") else {
                return true;
            };
            let mut owners = layouts.iter().filter_map(|(project, workspace)| {
                let tab = rest.strip_prefix(project.as_str())?.strip_prefix(':')?;
                Some(workspace.tabs.iter().any(|t| t.id == tab))
            });
            // Keep the key when no loaded project owns it, or when one of the
            // owners (ids may contain ':') still has the tab.
            owners.clone().next().is_none() || owners.any(|live| live)
        });
    }

    /// Live terminal sessions and open non-editor tabs under one project,
    /// grouped as a tree per top-level workspace tab in layout order.
    pub(super) fn project_sessions_tree(&mut self, ui: &mut egui::Ui, project_id: &str) {
        let mut live = self.live_project_sessions(project_id);
        let nested = super::child_rows::nested_ids(&self.state, &live);
        live.retain(|s| !nested.contains(&s.id));
        let live_ids: std::collections::HashSet<&str> =
            live.iter().map(|s| s.id.as_str()).collect();
        let (tab_groups, placed_sids) = self.project_tab_items(project_id, &live_ids);
        let floated = self.floated_non_session_tabs(project_id);
        let has_tab_items = tab_groups.iter().any(|(_, items)| !items.is_empty());
        if live.is_empty() && !has_tab_items && floated.is_empty() {
            return;
        }
        let by_id: std::collections::HashMap<&str, &Arc<Session>> =
            live.iter().map(|s| (s.id.as_str(), s)).collect();
        let overflow: Vec<String> = live
            .iter()
            .filter(|s| !placed_sids.contains(s.id.as_str()))
            .map(|s| s.id.clone())
            .collect();
        if tab_groups.len() < 2 {
            for (_, items) in &tab_groups {
                self.paint_flat_tree_items(ui, (project_id, items), &by_id);
            }
            self.paint_overflow_items(ui, (project_id, &overflow, &floated), &by_id);
            return;
        }
        let workspace_active = self.layouts.get(project_id).map(|w| w.active.clone());
        for (tab_id, items) in &tab_groups {
            if items.is_empty() {
                continue;
            }
            self.session_tab_group(
                ui,
                TabGroupInput {
                    project_id,
                    tab_id,
                    items,
                    by_id: &by_id,
                    workspace_active: workspace_active.as_deref(),
                },
            );
        }
        self.paint_overflow_items(ui, (project_id, &overflow, &floated), &by_id);
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

    /// Position and custom name of a workspace tab.
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
            items,
            by_id,
            workspace_active,
        } = input;
        let (group_index, group_name) = self.tab_group_name(project_id, tab_id);
        let renaming_group = self
            .rename_group
            .as_ref()
            .is_some_and(|rename| rename.project == *project_id && rename.tab == *tab_id);
        // One visible item and no custom name: render it directly so the
        // group header does not repeat the single item it holds.
        if items.len() == 1
            && group_name.is_none()
            && !renaming_group
            && let Some(first) = items.first()
        {
            self.paint_single_group_item(ui, (project_id, tab_id, first), by_id);
            return;
        }
        let group_label = match group_name.as_deref() {
            Some(name) => name.to_owned(),
            None => self.unnamed_tab_group_label(project_id, tab_id, group_index),
        };
        let key = tab_expanded_key(project_id, tab_id);
        let mut tab_expanded = self.preferences.expanded.get(&key).copied().unwrap_or(true);
        let active_tab = workspace_active == Some(tab_id.as_str());
        let states: Vec<Option<AgentState>> = items
            .iter()
            .filter_map(|item| match item {
                TreeItem::Session(sid) => Some(self.present_session(sid).lifecycle),
                TreeItem::Tab(_) => None,
            })
            .collect();
        let header = TabGroupHeader {
            project_id,
            tab_id,
            key: &key,
            label: &group_label,
            named: group_name.is_some(),
            renaming: renaming_group,
            selected: self.selected.as_deref() == Some(project_id) && active_tab,
            trailing: super::tree_dnd::group_activity(&states, items.len()),
            counts: crate::agent_presence::StateCounts::of(states.iter().flatten().copied()),
        };
        let (tab_left, rename_action) = ui
            .horizontal(|ui| self.tab_group_header(ui, &header, &mut tab_expanded))
            .inner;
        self.apply_group_rename((project_id, tab_id), group_name, rename_action);
        if tab_expanded {
            self.paint_group_children(ui, (project_id, tab_id, tab_left, items), by_id);
        }
    }

    fn paint_single_group_item(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, tab_id, item): (&str, &str, &TreeItem),
        by_id: &std::collections::HashMap<&str, &Arc<Session>>,
    ) {
        match item {
            TreeItem::Session(sid) => {
                if let Some(session) = by_id.get(sid.as_str()).map(|s| s.as_ref()) {
                    let response = self.session_row_in_tab(SessionRowInTabInput {
                        ui,
                        session,
                        project_id,
                        tab_id,
                    });
                    if let Some(response) = response {
                        Self::tree_session_row_dnd(&response, (project_id, &session.id));
                        self.tree_drop_target(ui, &response, (project_id, tab_id));
                    }
                    self.session_children(ui, project_id, &session.id);
                }
            }
            TreeItem::Tab(tab) => {
                if let Some(response) = self.sidebar_tab_row(ui, (project_id, Some(tab_id)), tab) {
                    self.tree_drop_target(ui, &response, (project_id, tab_id));
                }
            }
        }
    }

    fn paint_group_children(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, tab_id, tab_left, items): (&str, &str, f32, &[TreeItem]),
        by_id: &std::collections::HashMap<&str, &Arc<Session>>,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().indent += tab_left - ui.next_widget_position().x;
            ui.indent(format!("sidebar-tab-body:{tab_id}"), |ui| {
                for item in items {
                    match item {
                        TreeItem::Session(sid) => {
                            if let Some(session) = by_id.get(sid.as_str())
                                && let Some(response) = self.session_row(ui, session)
                            {
                                Self::tree_session_row_dnd(&response, (project_id, sid));
                            }
                            self.session_children(ui, project_id, sid);
                        }
                        TreeItem::Tab(tab) => {
                            self.sidebar_tab_row(ui, (project_id, None), tab);
                        }
                    }
                }
            });
        });
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
            // Expanded is the default: keep only collapsed tabs, so the
            // preferences do not gain a key per tab ever shown.
            if *tab_expanded {
                self.preferences.expanded.remove(header.key);
            } else {
                self.preferences
                    .expanded
                    .insert(header.key.to_owned(), false);
            }
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
        let height = self.theme.row_height();
        let Some(rename) = self.rename_group.as_mut() else {
            return (ui.next_widget_position().x, GroupRenameAction::None);
        };
        rename.painted = true;
        let focus = std::mem::take(&mut rename.focus);
        let field = ui.add_sized(
            [160.0, height],
            appearance::singleline(&mut rename.text)
                .id(ui.scope_id().with(("rename-group", header.tab_id.as_str())))
                .hint_text(header.label),
        );
        if focus {
            field.request_focus();
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
        // Agents show as state counters; without one, the terminal count.
        let counters = state_counters(header.counts, &self.theme);
        let response = appearance::project_row_with(
            appearance::ProjectRowInput {
                color: None,
                ui,
                label,
                icon: "LayoutDashboard",
                selected: header.selected,
                height: self.theme.row_height(),
                trailing: &header.trailing,
                tint: appearance::color(&self.theme.secondary),
            },
            appearance::RowExtras {
                counters: &counters,
                ..appearance::RowExtras::default()
            },
        )
        .on_hover_text(format!(
            "Tab group\n{label}\n{}\nDouble-click to rename",
            header.trailing
        ));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("sidebar-tab:{}", header.tab_id),
            response.rect,
        );
        Self::tree_drag_source(
            &response,
            super::tree_dnd::TreeDrag::Group {
                project: header.project_id.to_owned(),
                tab: header.tab_id.clone(),
            },
        );
        self.tree_drop_target(ui, &response, (header.project_id, header.tab_id));
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
                self.begin_group_rename(project_id, tab_id, group_name.unwrap_or_default());
            }
            GroupRenameAction::Commit => self.finish_group_rename(true),
            GroupRenameAction::Cancel => self.finish_group_rename(false),
            GroupRenameAction::Clear => {
                self.set_tab_group_name(project_id, tab_id, None);
                self.save_layouts();
            }
            GroupRenameAction::None => {}
        }
    }

    pub(crate) fn set_tab_group_name(
        &mut self,
        project_id: &str,
        tab_id: &str,
        name: Option<String>,
    ) {
        if let Some(group) = self
            .layouts
            .get_mut(project_id)
            .and_then(|w| w.tabs.iter_mut().find(|g| g.id == tab_id))
        {
            group.name = name;
        }
    }
}
