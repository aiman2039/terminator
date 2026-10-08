use super::super::agent_presence::attention_status_icon;
use super::attention::state_color;
use super::explorer_rows::ExplorerLocal;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    App, Job, Tab, appearance, icons,
    preferences::{HistorySort, ProjectSort},
    workspace_ops,
};
use eframe::egui::{self};
use std::sync::Arc;
use terminator_core::{AgentState, Session, SessionKind};

impl App {
    fn project_sort_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.preferences.project_sort;
        let menu = appearance::sidebar_action(ui, "ArrowDownWideNarrow", "Sort projects");
        egui::Popup::menu(&menu)
            .style(appearance::menu_style)
            .show(|ui| {
                for (sort, icon, target) in [
                    (ProjectSort::NameAsc, "ArrowDown", "sort-name-asc"),
                    (ProjectSort::NameDesc, "ArrowUp", "sort-name-desc"),
                    (
                        ProjectSort::LatestActivity,
                        "History",
                        "sort-latest-activity",
                    ),
                ] {
                    let check = if current == sort { "✓" } else { "" };
                    let response = appearance::menu_item(ui, sort.menu_label(), icon, check);
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), target, response.rect);
                    #[cfg(not(feature = "test-support"))]
                    let _ = target;
                    if response.clicked() {
                        self.preferences.project_sort = sort;
                        ui.close();
                    }
                }
            });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-sort", menu.rect);
        let _ = menu;
    }

    pub(super) fn history_sort_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let current = self.preferences.history_sort;
        let menu = appearance::menu_button(ui, "Sort", |ui| {
            for (sort, icon, target) in [
                (
                    HistorySort::LatestActivity,
                    "History",
                    "history-sort-latest-activity",
                ),
                (HistorySort::NameAsc, "ArrowDown", "history-sort-name-asc"),
                (HistorySort::NameDesc, "ArrowUp", "history-sort-name-desc"),
            ] {
                let check = if current == sort { "✓" } else { "" };
                let response = appearance::menu_item(ui, sort.menu_label(), icon, check);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), target, response.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = target;
                if response.clicked() {
                    self.preferences.history_sort = sort;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Sort History by latest activity or name");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "history-sort", menu.rect);
        let _ = menu;
    }

    fn removed_projects_menu(&mut self, ui: &mut egui::Ui) {
        let hidden: Vec<_> = self
            .state
            .projects
            .iter()
            .filter(|p| self.preferences.hidden_projects.contains(&p.id))
            .cloned()
            .collect();
        if hidden.is_empty() {
            return;
        }
        let menu = appearance::sidebar_action(ui, "Archive", "Removed projects");
        egui::Popup::menu(&menu)
            .style(appearance::menu_style)
            .show(|ui| {
                for project in hidden {
                    let response = appearance::menu_item(ui, &project.name, "FolderOpen", "")
                        .on_hover_text(project.path.display().to_string());
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("restore-project:{}", project.id),
                        response.rect,
                    );
                    if response.clicked() {
                        self.select_project(project.id);
                        ui.close();
                    }
                }
            });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "removed-projects", menu.rect);
        let _ = menu;
    }

    pub(super) fn op_root(&self) -> Option<std::path::PathBuf> {
        self.git_root()
            .or_else(|| self.cwd())
            .or_else(|| self.selected_project().map(|project| project.path.clone()))
    }

    pub(crate) fn queue_workspace(&mut self, op: workspace_ops::Op) {
        let Some(root) = self.op_root() else {
            self.error = Some("Open a project folder first.".into());
            return;
        };
        let _ = self.jobs.send(Job::Workspace(root, op));
    }

    pub(super) fn explorer_local(
        &mut self,
        path: &std::path::Path,
        local: ExplorerLocal,
        ui: &egui::Ui,
    ) {
        let root = self.op_root().unwrap_or_else(|| path.to_path_buf());
        match local {
            ExplorerLocal::Reveal => self.queue_workspace(workspace_ops::Op::Reveal(path.into())),
            ExplorerLocal::CopyRelative => {
                ui.ctx()
                    .copy_text(workspace_ops::relative_display(&root, path));
            }
            ExplorerLocal::Rename => {
                self.open_name_prompt(workspace_ops::NamePrompt::Rename {
                    from: path.into(),
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                });
            }
            ExplorerLocal::Duplicate => {
                self.queue_workspace(workspace_ops::Op::Duplicate(path.into()));
            }
            ExplorerLocal::Delete => self.pending_delete = Some(path.into()),
        }
    }

    pub(crate) fn projects(&mut self, ui: &mut egui::Ui) {
        // Pin the row height so the filter field shares one baseline with
        // the icon buttons.
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                self.removed_projects_menu(ui);
                self.project_sort_menu(ui);
                if self.has_worktrees() {
                    let worktree = appearance::sidebar_action(ui, "GitBranch", "Worktrees…");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "worktree-add", worktree.rect);
                    if worktree.clicked() {
                        self.worktree_open = true;
                    }
                }
                let add = appearance::sidebar_action(ui, "Plus", "New project");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "project-add", add.rect);
                if add.clicked() {
                    self.add_project = true;
                }
                // A standard TextEdit owns its keys natively (typing,
                // arrows, Cmd/Ctrl+A/C/X/V), and the global select-all
                // shortcut yields while any TextEdit is focused.
                let filter = ui.add_sized(
                    [ui.available_width(), appearance::TOOLBAR_BUTTON],
                    appearance::singleline(&mut self.preferences.project_filter)
                        .id(egui::Id::new("project-filter"))
                        .hint_text("Filter projects"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "project-filter", filter.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &filter;
            });
        });
        ui.spacing_mut().item_spacing.y = 0.0;
        let index = self.sidebar_index();
        let projects = self.cached_projects();
        let visible_ids: std::collections::HashSet<_> =
            projects.iter().map(|p| p.id.as_str()).collect();
        appearance::sidebar_scroll("projects").show(ui, |ui| {
            for p in projects.iter() {
                if index.parents.get(&p.id).is_some_and(|parents| {
                    parents
                        .iter()
                        .any(|parent| visible_ids.contains(parent.as_str()))
                }) {
                    continue;
                }
                ui.add_space(6.0);
                let count = index.live_counts.get(&p.id).copied().unwrap_or(0);
                let selected = self.selected.as_ref() == Some(&p.id);
                let mut expanded = *self
                    .preferences
                    .expanded
                    .entry(p.id.clone())
                    .or_insert(true);
                let project_left = ui
                    .horizontal(|ui| {
                        if ui
                            .add_sized(
                                [16.0, self.theme.row_height()],
                                egui::Button::image(
                                    egui::Image::new(icons::source(if expanded {
                                        "ChevronDown"
                                    } else {
                                        "ChevronRight"
                                    }))
                                    .tint(appearance::ICON_COLOR)
                                    .fit_to_exact_size(egui::vec2(12.0, 12.0)),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Expand or collapse project")
                            .clicked()
                        {
                            self.finish_rename(true);
                            expanded = !expanded;
                            self.preferences.expanded.insert(p.id.clone(), expanded);
                        }
                        let agent_face = self.project_agent_face(&p.id);
                        let response = if let Some((brand, state)) = agent_face {
                            appearance::session_row_spec(
                                ui,
                                appearance::SessionRowSpec {
                                    label: &p.name,
                                    icon: attention_status_icon(state),
                                    selected,
                                    trailing: &count.to_string(),
                                    tint: appearance::color(&self.theme.secondary),
                                    icon_tint: Some(state_color(state, &self.theme)),
                                    spin: state == AgentState::Running,
                                    subtitle: None,
                                    brand: Some(brand),
                                },
                            )
                        } else {
                            appearance::project_row(
                                ui,
                                &p.name,
                                if expanded { "FolderOpen" } else { "Folder" },
                                selected,
                                self.theme.row_height(),
                                &count.to_string(),
                                appearance::color(&self.theme.secondary),
                            )
                        }
                        .on_hover_text(format!(
                            "{}\n{}{}",
                            p.name,
                            p.path.display(),
                            self.state
                                .worktrees
                                .iter()
                                .find(|w| w.project_id == p.id)
                                .map(|w| if w.removed {
                                    "\nRemoved worktree; session history retained"
                                } else {
                                    "\nManaged Git worktree"
                                })
                                .unwrap_or("")
                        ));
                        #[cfg(feature = "test-support")]
                        diagnostics::record(
                            ui.ctx(),
                            &format!("project-row:{}", p.id),
                            response.rect,
                        );
                        if response.clicked() {
                            self.select_project(p.id.clone());
                        }
                        appearance::context_menu(&response, |ui| {
                            if self.has_worktrees()
                                && appearance::menu_item(ui, "New task worktree…", "GitBranch", "")
                                    .clicked()
                            {
                                self.select_project(p.id.clone());
                                self.open_worktree_wizard();
                                ui.close();
                            }
                            if self.has_worktrees() {
                                ui.separator();
                            }
                            if self.managed_worktree(&p.id).is_some()
                                && appearance::menu_item(ui, "Remove worktree…", "X", "").clicked()
                            {
                                self.confirm_remove_worktree(&p.id);
                                ui.close();
                            }
                            if appearance::menu_item(ui, "Remove project from sidebar", "X", "")
                                .clicked()
                            {
                                self.hide_project(&p.id);
                                ui.close();
                            }
                        });
                        response.rect.left()
                    })
                    .inner;
                if expanded && !self.preferences.hidden_projects.contains(&p.id) {
                    ui.scope(|ui| {
                        // Indent from the project row, past its separate expand button.
                        ui.spacing_mut().indent += project_left - ui.next_widget_position().x;
                        ui.indent(&p.id, |ui| {
                            self.project_sessions_tree(ui, &p.id);
                            if let Some(children) = index.children.get(&p.id) {
                                for child in children {
                                    self.worktree_card(ui, child);
                                }
                            }
                        });
                    });
                }
            }
            if projects.is_empty() {
                ui.weak(if self.state.projects.is_empty() {
                    "Add a folder to begin."
                } else if !self.preferences.project_filter.trim().is_empty() {
                    "No matching projects."
                } else {
                    "Restore a project from Removed."
                });
            }
        });
    }

    /// Live terminal sessions under one project, grouped as a tree per
    /// top-level workspace tab in layout order. Single-tab projects keep the
    /// historic flat list; multi-tab projects render one expandable tab node
    /// per tab that owns live sessions, then any background sessions that are
    /// in no tab.
    pub(super) fn project_sessions_tree(&mut self, ui: &mut egui::Ui, project_id: &str) {
        let index = self.sidebar_index();
        let live: Vec<Arc<Session>> = index
            .by_project
            .get(project_id)
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|s| s.lifecycle.live() && s.kind != SessionKind::Editor)
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
            let primary = self
                .layouts
                .get(project_id)
                .and_then(|workspace| workspace.tabs.iter().find(|group| &group.id == tab_id))
                .and_then(|group| {
                    group
                        .primary
                        .as_ref()
                        .filter(|tab| group.layout.find_tab(tab).is_some())
                        .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab))
                });
            let face = self.tab_face(primary);
            let key = format!("workspace-tab:{project_id}:{tab_id}");
            let mut tab_expanded = *self.preferences.expanded.entry(key.clone()).or_insert(true);
            let active_tab = workspace_active.as_deref() == Some(tab_id.as_str());
            let selected = self.selected.as_deref() == Some(project_id) && active_tab;
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
                                .tint(appearance::ICON_COLOR)
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
                    let response = appearance::project_row(
                        ui,
                        &face.label,
                        face.icon,
                        selected,
                        self.theme.row_height(),
                        &sids.len().to_string(),
                        appearance::color(&self.theme.secondary),
                    )
                    .on_hover_text(format!("Top-level tab\n{}", face.label));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("sidebar-tab:{tab_id}"), response.rect);
                    if response.clicked() {
                        self.select_project(project_id.to_string());
                        if let Some(workspace) = self.layouts.get_mut(project_id) {
                            workspace.active.clone_from(tab_id);
                        }
                        self.hide_center_overlay();
                    }
                    response.rect.left()
                })
                .inner;
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

/// Group live session ids into per-tab trees in layout order.
///
/// Each tab keeps its layout order and only ids present in `live` are kept;
/// tabs with no live sessions are skipped. Ids in no tab are returned as
/// overflow in `live` order (background sessions).
pub(crate) fn group_session_ids_by_tab(
    tabs: &[(String, Vec<String>)],
    live: &[String],
) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    use std::collections::HashSet;
    let known: HashSet<&str> = live.iter().map(String::as_str).collect();
    let mut in_tab: HashSet<&str> = HashSet::new();
    let mut groups = Vec::new();
    for (tab_id, sids) in tabs {
        let kept: Vec<String> = sids
            .iter()
            .filter(|sid| known.contains(sid.as_str()) && in_tab.insert(sid.as_str()))
            .cloned()
            .collect();
        if !kept.is_empty() {
            groups.push((tab_id.clone(), kept));
        }
    }
    let overflow: Vec<String> = live
        .iter()
        .filter(|sid| !in_tab.contains(sid.as_str()))
        .cloned()
        .collect();
    (groups, overflow)
}
