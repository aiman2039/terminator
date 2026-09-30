//! Project, file, Git, notification, and session-info sidebar rendering.
#[cfg(feature = "test-support")]
use crate::diagnostics;
#[cfg(any(test, target_os = "macos"))]
use crate::updater;
use crate::{
    App, Job, RenameSurface, Tab, appearance,
    file_actions::{self, FileAction},
    icons,
    preferences::{
        AgentsTab, ExplorerSearchMode, HistoryInput, HistorySort, ProjectSort, SidebarTool,
        VisibleProjects, sort_history, sort_visible_projects,
    },
    search,
    services::ContextData,
    session_info,
    settings_ui::SettingsSection,
    workspace_ops,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
};
use terminator_core::appearance::AppearanceConfig;
use terminator_core::{
    AgentState, NVIM_REVIEW_CAPABILITY, Notification, Project, Request, ReviewMode, Session,
    SessionKind, TERMINAL_NOTICES_CAPABILITY, now,
};

fn skip_clipped_git_row(ui: &mut egui::Ui) -> bool {
    let rect = egui::Rect::from_min_size(
        ui.next_widget_position(),
        egui::vec2(ui.available_width(), 24.0),
    );
    if ui.is_rect_visible(rect) {
        false
    } else {
        // egui scopes and allocate_space each consume one automatic ID.
        ui.allocate_space(rect.size());
        true
    }
}

impl App {
    pub(super) fn explorer_tooltip(&self) -> String {
        let Some(cwd) = self.cwd() else {
            return "Explorer — no selected directory".into();
        };
        let unconfirmed = self.context_session().is_some_and(|s| !s.cwd_confirmed);
        format!(
            "Explorer\n{}{}",
            cwd.display(),
            if unconfirmed {
                "\nLast known directory"
            } else {
                ""
            }
        )
    }
    pub(super) fn notifications(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let pending = self
                .state
                .notifications
                .iter()
                .filter(|n| notice_pending(n, now()))
                .count()
                + self
                    .state
                    .terminal_notices
                    .iter()
                    .filter(|n| !n.dismissed)
                    .count();
            let label = if pending == 0 {
                String::new()
            } else {
                pending.to_string()
            };
            let button = egui::Button::image_and_text(
                egui::Image::new(icons::source("Bell")).fit_to_exact_size(egui::vec2(16.0, 16.0)),
                label,
            )
            .frame(false);
            let response = ui
                .add(button)
                .on_hover_text(format!("{pending} pending notifications"));
            if response.clicked() {
                self.preferences.agents_tab = AgentsTab::NeedsAttention;
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "attention-bell", response.rect);
            let popup = egui::Popup::from_toggle_button_response(&response).show(|ui| {
                ui.set_width(300.0);
                ui.set_max_height(460.0);
                ui.push_id("attention-popup", |ui| self.agents_view(ui));
            });
            #[cfg(feature = "test-support")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("attention-state"), (pending, popup.is_some()))
            });
            let _ = popup;
        });
    }
    /// IDE status-bar mirror of the left agent bell: always visible in IDE
    /// mode so waiting/unread counts survive collapsed sidebars. Clicking
    /// reveals the Agents inbox in the right sidebar.
    pub(super) fn notification_status_badge(&mut self, ui: &mut egui::Ui) {
        let (waiting, unread) = self.attention_counts();
        let label = attention_badge_label(waiting, unread);
        let response = ui
            .add(
                egui::Button::image_and_text(
                    egui::Image::new(icons::source("Bell"))
                        .fit_to_exact_size(egui::vec2(14.0, 14.0)),
                    label,
                )
                .frame(false),
            )
            .on_hover_text("Pending agent notifications. Click to open the Agents inbox.");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-attention-bell", response.rect);
        if response.clicked() {
            self.preferences.tool = SidebarTool::Agents;
            self.preferences.visible = true;
            self.preferences.agents_tab = AgentsTab::NeedsAttention;
        }
    }

    pub(super) fn agent_bar(&mut self, ui: &mut egui::Ui) {
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                self.player_activity_row(ui);
            },
        );
        if self.player_chrome_expanded() {
            ui.spacing_mut().item_spacing.y = 4.0;
            self.player_live_controls(ui);
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(egui::Separator::default().spacing(4.0));
        ui.spacing_mut().item_spacing.y = spacing;
    }

    fn player_activity_row(&mut self, ui: &mut egui::Ui) {
        self.player_toggle_button(ui);
        let (waiting, unread) = self.attention_counts();
        let badge = attention_badge_label(waiting, unread);
        #[cfg(feature = "test-support")]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("agent-bar-badge"), badge.clone()));
        let response = appearance::row(
            ui,
            "",
            "Bell",
            self.preferences.left_agents,
            28.0,
            &badge,
            appearance::color(if waiting > 0 {
                &self.theme.status_waiting
            } else {
                &self.theme.text
            }),
        )
        .on_hover_text("Pending agent notifications. Click to switch between Agents and Projects.");
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Agents"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "left-agent-bar", response.rect);
        if response.clicked() {
            if !self.preferences.left_agents {
                self.preferences.agents_tab = AgentsTab::NeedsAttention;
            }
            self.preferences.left_agents = !self.preferences.left_agents;
        }
    }

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

    fn history_sort_menu(&mut self, ui: &mut egui::Ui) {
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

    fn sidebar_project(&self, project: &Project) -> std::sync::Arc<Project> {
        let mut cached = self.sidebar_projects.borrow_mut();
        let entry = cached
            .entry(project.id.clone())
            .or_insert_with(|| std::sync::Arc::new(project.clone()));
        if entry.as_ref() != project {
            *entry = std::sync::Arc::new(project.clone());
        }
        entry.clone()
    }

    pub(super) fn visible_projects(&self) -> Vec<std::sync::Arc<Project>> {
        sort_visible_projects(VisibleProjects {
            projects: &self.state.projects,
            hidden: &self.preferences.hidden_projects,
            sort: self.preferences.project_sort,
            activity: &self.preferences.project_activity,
            sessions: &self.state.sessions,
            agents: &self.state.agents,
            notifications: &self.state.notifications,
            terminal_notices: &self.state.terminal_notices,
        })
        .into_iter()
        .map(|project| self.sidebar_project(project))
        .collect()
    }
    fn op_root(&self) -> Option<std::path::PathBuf> {
        self.git_root()
            .or_else(|| self.cwd())
            .or_else(|| self.selected_project().map(|project| project.path.clone()))
    }

    pub(super) fn queue_workspace(&mut self, op: workspace_ops::Op) {
        let Some(root) = self.op_root() else {
            self.error = Some("Open a project folder first.".into());
            return;
        };
        let _ = self.jobs.send(Job::Workspace(root, op));
    }

    fn ensure_git_lists(&mut self, root: Option<std::path::PathBuf>) {
        let Some(root) = root else {
            return;
        };
        if self.git_list_root.as_ref() == Some(&root) {
            return;
        }
        self.git_list_root = Some(root);
        self.git_branches.clear();
        self.git_log.clear();
        self.git_compare = None;
        self.git_compare_root = None;
        self.git_base_ref = None;
        self.queue_workspace(workspace_ops::Op::Branches);
        self.queue_workspace(workspace_ops::Op::Log);
        self.queue_workspace(workspace_ops::Op::Compare { base: None });
    }

    fn explorer_local(&mut self, path: &std::path::Path, local: ExplorerLocal, ui: &egui::Ui) {
        let root = self.op_root().unwrap_or_else(|| path.to_path_buf());
        match local {
            ExplorerLocal::Reveal => self.queue_workspace(workspace_ops::Op::Reveal(path.into())),
            ExplorerLocal::CopyRelative => {
                ui.ctx()
                    .copy_text(workspace_ops::relative_display(&root, path));
            }
            ExplorerLocal::Rename => {
                self.name_prompt = Some(workspace_ops::NamePrompt::Rename {
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

    fn pending_delete_bar(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.pending_delete.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(format!(
                "Delete {}?",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            if ui.button("Delete").clicked() {
                self.queue_workspace(workspace_ops::Op::Delete(path));
                self.pending_delete = None;
            }
            if ui.button("Cancel").clicked() {
                self.pending_delete = None;
            }
        });
    }

    pub(super) fn explorer_toolbar(&mut self, ui: &mut egui::Ui, cwd: &std::path::Path) {
        let mut focus_find = false;
        // Pin the row height so the icon buttons, search field, and text
        // toggles all share one baseline.
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let cwd = cwd.to_path_buf();
                let more = appearance::compact_menu_button(ui, "…", |ui| {
                    if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "")
                        .clicked()
                    {
                        self.queue_workspace(workspace_ops::Op::Reveal(cwd.clone()));
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
                        ui.ctx().copy_text(cwd.display().to_string());
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked() {
                        let root = self.op_root().unwrap_or_else(|| cwd.clone());
                        ui.ctx()
                            .copy_text(workspace_ops::relative_display(&root, &cwd));
                        ui.close();
                    }
                    ui.separator();
                    if appearance::menu_item(ui, "Find in folder", "Search", "").clicked() {
                        focus_find = true;
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Collapse all", "ChevronsDownUp", "").clicked() {
                        self.expanded_dirs.clear();
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Folder actions");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "explorer-more", more.rect);
                let _ = more;
                let ignored_icon = if self.preferences.show_ignored {
                    "Eye"
                } else {
                    "EyeOff"
                };
                let ignored = appearance::selectable_icon(
                    ui,
                    ignored_icon,
                    "Show ignored files (excluded by Git ignore rules and Git metadata)",
                    self.preferences.show_ignored,
                );
                let refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh");
                let collapse = appearance::sidebar_action(ui, "ChevronsDownUp", "Collapse all");
                let new_folder = appearance::sidebar_action(ui, "Folder", "New folder");
                let new_file = appearance::sidebar_action(ui, "File", "New file");
                #[cfg(feature = "test-support")]
                {
                    diagnostics::record(ui.ctx(), "explorer-new-file", new_file.rect);
                    diagnostics::record(ui.ctx(), "explorer-new-folder", new_folder.rect);
                    diagnostics::record(ui.ctx(), "explorer-collapse", collapse.rect);
                    diagnostics::record(ui.ctx(), "explorer-refresh", refresh.rect);
                    diagnostics::record(ui.ctx(), "explorer-show-ignored", ignored.rect);
                }
                if ignored.clicked() {
                    self.preferences.show_ignored = !self.preferences.show_ignored;
                    self.explorer_search_last = None;
                }
                if new_file.clicked() {
                    self.name_prompt = Some(workspace_ops::NamePrompt::File {
                        dir: cwd.clone(),
                        name: String::new(),
                    });
                }
                if new_folder.clicked() {
                    self.name_prompt = Some(workspace_ops::NamePrompt::Folder {
                        dir: cwd.clone(),
                        name: String::new(),
                    });
                }
                if collapse.clicked() {
                    self.expanded_dirs.clear();
                }
                if refresh.clicked() {
                    self.refresh_request = None;
                }
            });
        });
        let contents = self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let icon = 18.0;
            ui.add_sized(
                [icon, icon],
                egui::Image::new(icons::source("Search"))
                    .tint(appearance::color(&self.theme.secondary)),
            );
            let toggles = if contents { 78.0 } else { 0.0 };
            let find = ui.add_sized(
                [ui.available_width() - toggles, appearance::TOOLBAR_BUTTON],
                egui::TextEdit::singleline(&mut self.explorer_query).hint_text(if contents {
                    "Search"
                } else {
                    "Find in folder"
                }),
            );
            if focus_find {
                find.request_focus();
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-search", find.rect);
            if contents {
                let case = explorer_toggle(
                    ui,
                    "Aa",
                    "Match case",
                    self.preferences.explorer_match_case,
                    "explorer-match-case",
                );
                let word = explorer_toggle(
                    ui,
                    "ab",
                    "Match whole word",
                    self.preferences.explorer_whole_word,
                    "explorer-whole-word",
                );
                let regex = explorer_toggle(
                    ui,
                    ".*",
                    "Use regular expression",
                    self.preferences.explorer_regex,
                    "explorer-regex",
                );
                if case.clicked() {
                    self.preferences.explorer_match_case = !self.preferences.explorer_match_case;
                }
                if word.clicked() {
                    self.preferences.explorer_whole_word = !self.preferences.explorer_whole_word;
                }
                if regex.clicked() {
                    self.preferences.explorer_regex = !self.preferences.explorer_regex;
                }
            }
        });
        ui.add_space(2.0);
        explorer_segmented(ui, &mut self.preferences.explorer_search_mode);
        if contents {
            ui.add_space(4.0);
            ui.label(
                RichText::new("FILES TO INCLUDE")
                    .small()
                    .color(appearance::color(&self.theme.secondary)),
            );
            let include = ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.preferences.explorer_include)
                    .hint_text("files to include (e.g. *.ts, src/**)"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-include", include.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &include;
            ui.add_space(4.0);
            ui.label(
                RichText::new("FILES TO EXCLUDE")
                    .small()
                    .color(appearance::color(&self.theme.secondary)),
            );
            let exclude = ui.add_sized(
                [ui.available_width(), 24.0],
                egui::TextEdit::singleline(&mut self.preferences.explorer_exclude)
                    .hint_text("files to exclude (e.g. *.min.js, dist/**)"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-exclude", exclude.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &exclude;
        }
        self.refresh_explorer_search(cwd);
        self.name_prompt_bar(ui);
    }

    /// Queue a Contents search when the query or filters changed. Results are
    /// tagged with a generation so stale workers are dropped.
    fn refresh_explorer_search(&mut self, cwd: &std::path::Path) {
        if self.preferences.explorer_search_mode != ExplorerSearchMode::Contents {
            return;
        }
        let query = search::Query {
            text: self.explorer_query.clone(),
            match_case: self.preferences.explorer_match_case,
            whole_word: self.preferences.explorer_whole_word,
            use_regex: self.preferences.explorer_regex,
            include: self.preferences.explorer_include.clone(),
            exclude: self.preferences.explorer_exclude.clone(),
        };
        if self.explorer_search_last.as_ref() == Some(&query) {
            return;
        }
        self.explorer_search_last = Some(query.clone());
        self.explorer_search_generation = self.explorer_search_generation.wrapping_add(1);
        if query.is_empty() {
            self.explorer_search.clear();
            self.explorer_search_error = None;
            self.explorer_search_pending = false;
            return;
        }
        self.explorer_search_pending = true;
        let id = self.explorer_search_generation;
        let show_ignored = self.preferences.show_ignored;
        let _ = self.jobs.send(Job::Search {
            id,
            root: cwd.to_path_buf(),
            query,
            show_ignored,
        });
    }

    /// Contents search results, grouped by file. Clicking a hit opens that file.
    fn explorer_results(&mut self, ui: &mut egui::Ui, cwd: &std::path::Path) {
        if let Some(error) = &self.explorer_search_error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            return;
        }
        if self.explorer_search.is_empty() {
            ui.weak(if self.explorer_search_pending {
                "Searching…"
            } else if self.explorer_query.trim().is_empty() {
                "Type to search in files"
            } else {
                "No results"
            });
            return;
        }
        if self.explorer_search_pending {
            ui.weak("Searching…");
        }
        let root = self.op_root().unwrap_or_else(|| cwd.to_path_buf());
        let mut open: Option<PathBuf> = None;
        let mut last: Option<&Path> = None;
        for hit in &self.explorer_search {
            if skip_clipped_git_row(ui) {
                continue;
            }
            if last != Some(hit.path.as_path()) {
                let relative = workspace_ops::relative_display(&root, &hit.path);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(relative)
                            .small()
                            .color(appearance::color(&self.theme.secondary)),
                    )
                    .truncate(),
                )
                .on_hover_text(hit.path.display().to_string());
                last = Some(hit.path.as_path());
            }
            let response = appearance::file_row(
                ui,
                &hit.text,
                icons::file_icon(&hit.path),
                false,
                20.0,
                &hit.line.to_string(),
                appearance::color(&self.theme.text),
            )
            .on_hover_text(format!("{}:{}", hit.path.display(), hit.line));
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("search-hit:{}:{}", hit.path.display(), hit.line),
                response.rect,
            );
            if response.clicked() {
                open = Some(hit.path.clone());
            }
        }
        if let Some(path) = open {
            self.activate_file_action(ui, &path, FileAction::Open);
        }
    }

    fn name_prompt_bar(&mut self, ui: &mut egui::Ui) {
        let Some(prompt) = self.name_prompt.clone() else {
            return;
        };
        let title = match prompt {
            workspace_ops::NamePrompt::File { .. } => "New file",
            workspace_ops::NamePrompt::Folder { .. } => "New folder",
            workspace_ops::NamePrompt::Rename { .. } => "Rename",
        };
        let mut name = match &prompt {
            workspace_ops::NamePrompt::File { name, .. }
            | workspace_ops::NamePrompt::Folder { name, .. }
            | workspace_ops::NamePrompt::Rename { name, .. } => name.clone(),
        };
        let mut cancel = false;
        let mut submit = false;
        ui.horizontal(|ui| {
            ui.label(title);
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text("Name")
                    .desired_width(140.0),
            );
            submit = ui.button("Save").clicked()
                || (field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            cancel = ui.button("Cancel").clicked();
        });
        if cancel {
            self.name_prompt = None;
            return;
        }
        self.name_prompt = Some(match prompt {
            workspace_ops::NamePrompt::File { dir, .. } => {
                workspace_ops::NamePrompt::File { dir, name }
            }
            workspace_ops::NamePrompt::Folder { dir, .. } => {
                workspace_ops::NamePrompt::Folder { dir, name }
            }
            workspace_ops::NamePrompt::Rename { from, .. } => {
                workspace_ops::NamePrompt::Rename { from, name }
            }
        });
        if submit {
            self.submit_name_prompt();
        }
    }

    fn submit_name_prompt(&mut self) {
        let Some(prompt) = self.name_prompt.clone() else {
            return;
        };
        let op = match prompt {
            workspace_ops::NamePrompt::File { dir, name } => {
                workspace_ops::Op::CreateFile(dir.join(name.trim()))
            }
            workspace_ops::NamePrompt::Folder { dir, name } => {
                workspace_ops::Op::CreateDir(dir.join(name.trim()))
            }
            workspace_ops::NamePrompt::Rename { from, name } => {
                let Some(parent) = from.parent().map(std::path::Path::to_path_buf) else {
                    return;
                };
                workspace_ops::Op::Rename {
                    from,
                    to: parent.join(name.trim()),
                }
            }
        };
        self.name_prompt = None;
        self.queue_workspace(op);
    }

    pub(super) fn projects(&mut self, ui: &mut egui::Ui) {
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
            });
        });
        ui.spacing_mut().item_spacing.y = 0.0;
        let live = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live() && s.kind != SessionKind::Editor)
            .count();
        let footer = 28.0;
        let projects = self.visible_projects();
        appearance::sidebar_scroll("projects")
            .max_height((ui.available_height() - footer).max(0.0))
            .show(ui, |ui| {
                for p in &projects {
                    if self.managed_worktree(&p.id).is_some()
                        && projects.iter().any(|parent| {
                            self.worktree_children(parent)
                                .iter()
                                .any(|worktree| worktree.project_id == p.id)
                        })
                    {
                        continue;
                    }
                    ui.add_space(6.0);
                    let count = self
                        .state
                        .sessions
                        .iter()
                        .filter(|s| {
                            s.project_id == p.id
                                && s.lifecycle.live()
                                && s.kind != SessionKind::Editor
                        })
                        .count();
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
                                    && appearance::menu_item(
                                        ui,
                                        "New task worktree…",
                                        "GitBranch",
                                        "",
                                    )
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
                                    && appearance::menu_item(ui, "Remove worktree…", "X", "")
                                        .clicked()
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
                                let sessions: Vec<_> = self
                                    .state
                                    .sessions
                                    .iter()
                                    .filter(|s| {
                                        s.project_id == p.id && s.kind != SessionKind::Editor
                                    })
                                    .cloned()
                                    .collect();
                                for session in sessions
                                    .iter()
                                    .filter(|s| s.lifecycle.live() && s.kind != SessionKind::Editor)
                                {
                                    self.session_row(ui, session);
                                }
                                let children: Vec<_> = self
                                    .worktree_children(p)
                                    .into_iter()
                                    .map(|worktree| worktree.project_id.clone())
                                    .collect();
                                for child_id in children {
                                    if let Some(child) = self
                                        .state
                                        .projects
                                        .iter()
                                        .find(|project| project.id == child_id)
                                        .cloned()
                                    {
                                        self.worktree_card(ui, &child);
                                    }
                                }
                            });
                        });
                    }
                }
                if self
                    .state
                    .projects
                    .iter()
                    .all(|p| self.preferences.hidden_projects.contains(&p.id))
                {
                    ui.weak(if self.state.projects.is_empty() {
                        "Add a folder to begin."
                    } else {
                        "Restore a project from Removed."
                    });
                }
            });
        ui.add_space((ui.available_height() - footer).max(0.0));
        ui.separator();
        ui.weak(format!("{live} live sessions")).on_hover_text("Sessions continue when this window closes. Ended sessions remain in History until removed.");
    }
    /// Brand plus status for a project row when one of its live sessions has an agent.
    /// A running agent wins over a finished one, so a working logo is not replaced by a folder.
    fn project_agent_face(&self, project_id: &str) -> Option<(&'static str, AgentState)> {
        let mut best: Option<(&'static str, AgentState, u8)> = None;
        for session in self.state.sessions.iter().filter(|session| {
            session.project_id == project_id
                && session.lifecycle.live()
                && session.kind != SessionKind::Editor
        }) {
            let presented = self.present_session(&session.id);
            let Some(brand) = presented.brand_icon else {
                continue;
            };
            let state = presented.lifecycle.unwrap_or(AgentState::Unknown);
            let rank = match state {
                AgentState::Running => 0,
                AgentState::WaitingInput | AgentState::WaitingPermission => 1,
                AgentState::Failed => 2,
                AgentState::Completed => 3,
                _ => 4,
            };
            if best.is_none_or(|(_, _, previous)| rank < previous) {
                best = Some((brand, state, rank));
            }
        }
        best.map(|(brand, state, _)| (brand, state))
    }

    fn session_row(&mut self, ui: &mut egui::Ui, session: &Session) {
        let presented = self.present_session(&session.id);
        let terminal_note = self
            .state
            .terminal_notices
            .iter()
            .rev()
            .find(|n| n.session_id == session.id);
        let unread_terminal = terminal_note.is_some_and(|n| !n.dismissed);
        let color = appearance::color(if unread_terminal {
            &self.theme.accent
        } else {
            &self.theme.secondary
        });
        let visible = self
            .layouts
            .get(&session.project_id)
            .is_some_and(|d| d.contains(&Tab::Terminal(session.id.clone())));
        let secondary = if !session.lifecycle.live() {
            "ended"
        } else if !visible {
            "background"
        } else {
            ""
        };
        let editing = self.renaming(&session.id, RenameSurface::Sidebar);
        // The status icon carries agent state; the dot is for terminal notices.
        let trailing = if editing {
            ""
        } else if !secondary.is_empty() {
            secondary
        } else if unread_terminal {
            "●"
        } else {
            ""
        };
        let response = appearance::session_row_spec(
            ui,
            appearance::SessionRowSpec {
                label: if editing { "" } else { &session.label },
                icon: match presented.lifecycle {
                    Some(_) => presented.status_icon,
                    None if session.kind == SessionKind::Editor => "FileCode",
                    None => "Terminal",
                },
                selected: self.active_session.as_ref() == Some(&session.id),
                trailing,
                tint: color,
                icon_tint: presented
                    .lifecycle
                    .map(|state| state_color(state, &self.theme)),
                spin: presented.spin,
                subtitle: presented.notice_preview.as_deref().filter(|_| !editing),
                brand: presented.brand_icon,
            },
        )
        .on_hover_ui(|ui| {
            ui.label(format!(
                "{}{}{}",
                presented.diagnostics(now()),
                if secondary.is_empty() {
                    String::new()
                } else {
                    format!("\n{secondary}")
                },
                terminal_note
                    .map(|n| format!("\nTerminal: {}\n{}", n.title, n.body))
                    .unwrap_or_default()
            ));
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("session-row:{}", session.id),
            response.rect,
        );
        if editing {
            self.inline_rename(
                ui,
                &session.id,
                RenameSurface::Sidebar,
                egui::Rect::from_min_max(
                    response.rect.min + egui::vec2(26.0, 4.0),
                    response.rect.max - egui::vec2(6.0, 3.0),
                ),
            );
        }
        if response.clicked() && !editing {
            self.go_session(&session.id);
        }
        appearance::context_menu(&response, |ui| {
            self.rename_action(ui, &session.id, RenameSurface::Sidebar);
            ui.separator();
            if appearance::menu_item(ui, "Open session", "Terminal", "").clicked() {
                self.go_session(&session.id);
                ui.close();
            }
            ui.separator();
            if session.lifecycle.live() {
                if appearance::menu_item(
                    ui,
                    "Close session…",
                    "X",
                    &self.shortcut_label("close_session"),
                )
                .clicked()
                {
                    self.close_session = Some(session.id.clone());
                    ui.close();
                }
            } else if appearance::menu_item(ui, "Remove historical record", "X", "").clicked() {
                self.send(Request::Remove {
                    session: session.id.clone(),
                });
                self.remove_tab(&session.id);
                ui.close();
            }
        });
    }
    fn worktree_card(&mut self, ui: &mut egui::Ui, project: &Project) {
        let live = self
            .state
            .sessions
            .iter()
            .filter(|session| {
                session.project_id == project.id
                    && session.lifecycle.live()
                    && session.kind != SessionKind::Editor
            })
            .count();
        let mut waiting = false;
        let mut running = false;
        for session in self
            .state
            .sessions
            .iter()
            .filter(|session| session.project_id == project.id && session.lifecycle.live())
        {
            match self.present_session(&session.id).lifecycle {
                Some(AgentState::WaitingInput | AgentState::WaitingPermission) => waiting = true,
                Some(AgentState::Running) => running = true,
                _ => {}
            }
        }
        let tint = appearance::color(if waiting {
            &self.theme.status_waiting
        } else if running {
            &self.theme.status_running
        } else {
            &self.theme.secondary
        });
        let count = live.to_string();
        let selected = self.selected.as_ref() == Some(&project.id);
        let response = appearance::project_row(
            ui,
            &project.name,
            "GitBranch",
            selected,
            self.theme.row_height(),
            &count,
            tint,
        )
        .on_hover_text(format!(
            "{}\nManaged Git worktree\n{} live terminal(s)",
            project.path.display(),
            live
        ));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("worktree-row:{}", project.id),
            response.rect,
        );
        if response.clicked() {
            self.select_project(project.id.clone());
        }
        appearance::context_menu(&response, |ui| {
            if appearance::menu_item(ui, "Open", "FolderOpen", "").clicked() {
                self.select_project(project.id.clone());
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "New terminal",
                "Terminal",
                &self.shortcut_label("new_terminal"),
            )
            .clicked()
            {
                self.select_project(project.id.clone());
                self.create(None);
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(ui, "Remove worktree…", "X", "").clicked() {
                self.confirm_remove_worktree(&project.id);
                ui.close();
            }
        });
    }
    pub(super) fn tree(&mut self, ui: &mut egui::Ui, path: &std::path::Path, depth: usize) {
        ui.spacing_mut().interact_size.y = 24.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        if depth > 20 {
            return;
        }
        self.visible_dirs.push(path.into());
        if let Some(error) = self.directory_errors.get(path).cloned() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!(
                    "Cannot refresh {} ({:?}): {}",
                    error.path.display(),
                    error.kind,
                    error.message
                ),
            );
            if self.dirs.contains_key(path) {
                ui.weak("Showing the last successful listing.");
            }
            ui.horizontal(|ui| {
                let retry = ui.button("Retry");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "directory-retry", retry.rect);
                if retry.clicked() {
                    self.refresh_request = None;
                }
                if ui.button("Choose folder again").clicked() {
                    self.add_project = true;
                }
            });
            if cfg!(target_os = "macos") {
                ui.weak("Check System Settings → Privacy & Security → Files and Folders. Full Disk Access is optional troubleshooting; this error may have another cause. Shells and editors can have separate access.");
            }
        }
        let entries = self.dirs.get(path).cloned();
        if let Some(entries) = entries {
            for entry in entries {
                if entry.ignored && !self.preferences.show_ignored {
                    continue;
                }
                let label = entry
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !entry.directory
                    && !self.explorer_query.is_empty()
                    && !label
                        .to_lowercase()
                        .contains(&self.explorer_query.to_lowercase())
                {
                    continue;
                }
                if entry.directory {
                    let expanded = self.expanded_dirs.contains(&entry.path);
                    let status = self
                        .context
                        .as_ref()
                        .and_then(|c| c.decorations.get(&entry.path))
                        .copied()
                        .unwrap_or(' ');
                    let color = if entry.ignored {
                        appearance::color(&self.theme.git_ignored)
                    } else {
                        git_color(&self.theme, status)
                    };
                    let folder = appearance::file_row(
                        ui,
                        &label,
                        if expanded { "FolderOpen" } else { "Folder" },
                        false,
                        22.0,
                        &status.to_string(),
                        color,
                    )
                    .on_hover_text(format!(
                        "{}\n{}",
                        entry.path.display(),
                        if entry.ignored {
                            "Ignored"
                        } else {
                            terminator_git::status_description(status)
                        }
                    ));
                    if folder.clicked() {
                        if expanded {
                            self.expanded_dirs.remove(&entry.path);
                        } else {
                            self.expanded_dirs.insert(entry.path.clone());
                        }
                    }
                    let folder_path = entry.path.clone();
                    appearance::context_menu(&folder, |ui| {
                        if appearance::menu_item(ui, "New file", "File", "").clicked() {
                            self.name_prompt = Some(workspace_ops::NamePrompt::File {
                                dir: folder_path.clone(),
                                name: String::new(),
                            });
                            ui.close();
                        }
                        if appearance::menu_item(ui, "New folder", "Folder", "").clicked() {
                            self.name_prompt = Some(workspace_ops::NamePrompt::Folder {
                                dir: folder_path.clone(),
                                name: String::new(),
                            });
                            ui.close();
                        }
                        if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "")
                            .clicked()
                        {
                            self.queue_workspace(workspace_ops::Op::Reveal(folder_path.clone()));
                            ui.close();
                        }
                        if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
                            ui.ctx().copy_text(folder_path.display().to_string());
                            ui.close();
                        }
                        ui.separator();
                        if appearance::menu_item(ui, "Collapse all", "ChevronDown", "").clicked() {
                            self.expanded_dirs.clear();
                            ui.close();
                        }
                    });
                    if expanded {
                        ui.indent(&entry.path, |ui| self.tree(ui, &entry.path, depth + 1));
                    }
                } else {
                    let status = self
                        .context
                        .as_ref()
                        .and_then(|c| c.decorations.get(&entry.path))
                        .copied()
                        .unwrap_or(' ');
                    let open_shortcut = self.shortcut_label("open_file");
                    let split_shortcut = self.shortcut_label("split_right");
                    let outcome = explorer_file_row(
                        ui,
                        &entry.path,
                        &label,
                        status,
                        entry.ignored,
                        &self.theme,
                        &open_shortcut,
                        &split_shortcut,
                    );
                    if let Some(action) = outcome.clicked {
                        self.activate_file_action(ui, &entry.path, action);
                    }
                    if let Some(action) = outcome.menu {
                        self.file_action(ui, action, &entry.path, None);
                    }
                    if let Some(local) = outcome.local {
                        self.explorer_local(&entry.path, local, ui);
                    }
                }
            }
        } else if !self.directory_errors.contains_key(path) {
            ui.weak("Loading…");
        }
    }
    pub(super) fn agents_inbox_open(&self) -> bool {
        self.preferences.left_agents
            || (self.preferences.visible && self.preferences.tool == SidebarTool::Agents)
    }

    // Inline selection is not a modal and must not take terminal keyboard focus.
    pub(super) fn notice_detail_modal_open(&self) -> bool {
        self.detail.as_ref().is_some_and(|id| {
            self.state
                .notifications
                .iter()
                .find(|notice| &notice.id == id)
                .is_some_and(|notice| {
                    !self.agents_inbox_open()
                        || notice.dismissed
                        || notice.resolved
                        || notice.snoozed_until > now()
                        || !self.notice_in_scope(notice, self.selected.as_deref())
                })
        })
    }

    pub(super) fn agents_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agents");
        // The tab bar scrolls instead of widening narrow sidebars.
        egui::ScrollArea::horizontal()
            .id_salt("agents-tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, icon, label, name) in [
                        (
                            AgentsTab::NeedsAttention,
                            "CircleAlert",
                            "Needs attention",
                            "needs",
                        ),
                        (AgentsTab::AllLive, "CircleCheck", "All live", "live"),
                        (AgentsTab::Unread, "Bell", "Unread", "unread"),
                    ] {
                        let selected = self.preferences.agents_tab == tab;
                        let response = appearance::selectable_icon(ui, icon, label, selected);
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), &format!("agent-tab:{name}"), response.rect);
                        #[cfg(not(feature = "test-support"))]
                        let _ = name;
                        if response.clicked() {
                            self.preferences.agents_tab = tab;
                            self.unread_selected = None;
                        }
                    }
                });
            });
        match self.preferences.agents_tab {
            AgentsTab::NeedsAttention => self.agents_needs_attention(ui),
            AgentsTab::AllLive => self.agents_all_live(ui),
            AgentsTab::Unread => self.agents_unread(ui),
        }
    }

    /// The existing inbox: pending notifications plus terminal notices.
    fn agents_needs_attention(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.preferences.all_projects, "All projects");
        let notices = self.pending_notices();
        let terminal_notices: Vec<_> = self
            .state
            .terminal_notices
            .iter()
            .filter(|notice| {
                !notice.dismissed
                    && self.state.sessions.iter().any(|session| {
                        session.id == notice.session_id
                            && self
                                .preferences
                                .includes_project(&session.project_id, self.selected.as_deref())
                    })
            })
            .rev()
            .cloned()
            .collect();
        let waiting = self.waiting_notice_count();
        if waiting > 0 {
            ui.label(
                RichText::new(format!("{waiting} waiting for action"))
                    .color(appearance::color(&self.theme.status_waiting)),
            );
        }
        appearance::sidebar_scroll("agents").show(ui, |ui| {
            if notices.is_empty() && terminal_notices.is_empty() {
                self.agents_empty(ui);
            }
            for group in group_notices(notices) {
                let Some(notice) = group.notices.first() else {
                    continue;
                };
                let session = self
                    .state
                    .sessions
                    .iter()
                    .find(|session| session.id == notice.session_id)
                    .cloned();
                let highlight = group
                    .notices
                    .iter()
                    .any(|n| self.detail.as_ref() == Some(&n.id));
                let selected = self.active_session.as_ref() == Some(&notice.session_id);
                let presented = self.present_session(&notice.session_id);
                let action = attention_card(
                    ui,
                    AttentionCard {
                        theme: &self.theme,
                        notice,
                        session: session.as_ref(),
                        selected,
                        highlight,
                        brand_icon: presented.brand_icon,
                        brand_label: presented.brand_label.as_deref(),
                        show_read: false,
                        group_extra: &group.notices[1..],
                    },
                );
                self.apply_group_action(&group, action);
            }
            for notice in terminal_notices {
                let row = ui.group(|ui| {
                    ui.label(if notice.title.is_empty() {
                        "Terminal"
                    } else {
                        &notice.title
                    });
                    ui.label(&notice.body);
                    ui.horizontal(|ui| {
                        let go = ui.button("Go to terminal");
                        #[cfg(feature = "test-support")]
                        diagnostics::record(
                            ui.ctx(),
                            &format!("terminal-go:{}", notice.session_id),
                            go.rect,
                        );
                        if go.clicked() {
                            self.go_session(&notice.session_id);
                        }
                        if self
                            .state
                            .capabilities
                            .iter()
                            .any(|c| c == TERMINAL_NOTICES_CAPABILITY)
                            && ui.button("Dismiss").clicked()
                        {
                            self.send(Request::DismissTerminalNotice {
                                id: notice.id.clone(),
                            });
                        }
                    });
                });
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("terminal-row:{}", notice.session_id),
                    row.response.rect,
                );
                let _ = row;
            }
        });
    }

    pub(super) fn unread_notices(&self) -> Vec<Notification> {
        let selected = self.selected.as_deref();
        let mut notices: Vec<_> = self
            .state
            .notifications
            .iter()
            .filter(|notice| {
                !notice.dismissed
                    && !notice.resolved
                    && notice.snoozed_until <= now()
                    && self.notice_in_scope(notice, selected)
                    && (!notice.read || self.unread_selected.as_deref() == Some(notice.id.as_str()))
            })
            .cloned()
            .collect();
        notices.sort_by_key(|notice| Reverse(notice.created));
        notices
    }

    /// Read-state inbox. Marking read never resolves, dismisses, or changes
    /// lifecycle; the selected row stays until selection or filter changes.
    fn agents_unread(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.preferences.all_projects, "All projects")
            .changed()
        {
            self.unread_selected = None;
        }
        let notices = self.unread_notices();
        appearance::sidebar_scroll("agents-unread").show(ui, |ui| {
            if notices.is_empty() {
                ui.weak("No unread agent events");
                return;
            }
            for group in group_notices(notices) {
                let Some(notice) = group.notices.first() else {
                    continue;
                };
                let session = self
                    .state
                    .sessions
                    .iter()
                    .find(|session| session.id == notice.session_id)
                    .cloned();
                let highlight = group.notices.iter().any(|n| {
                    self.detail.as_ref() == Some(&n.id)
                        || self.unread_selected.as_deref() == Some(n.id.as_str())
                });
                let selected = self.active_session.as_ref() == Some(&notice.session_id);
                let presented = self.present_session(&notice.session_id);
                let action = attention_card(
                    ui,
                    AttentionCard {
                        theme: &self.theme,
                        notice,
                        session: session.as_ref(),
                        selected,
                        highlight,
                        brand_icon: presented.brand_icon,
                        brand_label: presented.brand_label.as_deref(),
                        show_read: true,
                        group_extra: &group.notices[1..],
                    },
                );
                self.apply_group_action(&group, action);
            }
        });
    }

    /// Verified live agents grouped by project/worktree, plus hook-only and
    /// unavailable-owner entries under Presence unverified.
    fn agents_all_live(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() - 132.0).max(80.0);
            let response = ui.add_sized(
                egui::vec2(width, 22.0),
                egui::TextEdit::singleline(&mut self.preferences.agents_search)
                    .id(egui::Id::new("agents-search"))
                    .hint_text("Search agents"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "agents-search", response.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &response;
            if !self.preferences.agents_search.is_empty()
                && ui.small_button("✕").on_hover_text("Clear search").clicked()
            {
                self.preferences.agents_search.clear();
            }
            self.agents_filter_menu(ui);
        });
        let query = self.preferences.agents_search.trim().to_lowercase();
        let kind_filter = self.preferences.agents_filter.clone();
        let live: Vec<_> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .filter_map(|s| {
                let p = self.present_session(&s.id);
                p.live.then(|| (s, p.detected_kinds.clone()))
            })
            .collect();
        let unverified: Vec<_> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .filter(|s| {
                let presented = self.present_session(&s.id);
                !presented.live && presented.lifecycle.is_some()
            })
            .collect();
        let matches = |session: &Session, kinds: &[String]| -> bool {
            if !kind_filter.is_empty() && !kinds.iter().any(|k| k == &kind_filter) {
                return false;
            }
            if query.is_empty() {
                return true;
            }
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)
                .map(|p| p.name.to_lowercase())
                .unwrap_or_default();
            let presented = self.present_session(&session.id);
            let mut haystack = format!(
                "{} {project} {}",
                session.label.to_lowercase(),
                presented.status_label.to_lowercase()
            );
            for kind in kinds {
                haystack.push(' ');
                haystack.push_str(&terminator_core::agents::display_name(kind).to_lowercase());
            }
            query.split_whitespace().all(|word| haystack.contains(word))
        };
        let live: Vec<(Session, Vec<String>)> = live
            .into_iter()
            .filter(|(session, kinds)| matches(session, kinds))
            .map(|(session, kinds)| (session.clone(), kinds))
            .collect();
        let unverified: Vec<Session> = unverified
            .into_iter()
            .filter(|session| {
                let kinds: Vec<String> = self
                    .state
                    .agents
                    .iter()
                    .filter(|a| a.session_id == session.id)
                    .map(|a| a.kind.clone())
                    .collect();
                matches(session, &kinds)
            })
            .cloned()
            .collect();
        appearance::sidebar_scroll("agents-live").show(ui, |ui| {
            if live.is_empty() && unverified.is_empty() {
                ui.weak(if query.is_empty() && kind_filter.is_empty() {
                    "No live agents"
                } else {
                    "No matching agents"
                });
                return;
            }
            // Stable project order; worktree checkouts are their own groups.
            let groups: Vec<_> = self.state.projects.iter()
                .filter(|p| live.iter().any(|(s, _)| s.project_id == p.id))
                .map(|p| self.sidebar_project(p))
                .collect();
            for project in groups {
                let sessions: Vec<_> = live
                    .iter()
                    .filter(|(session, _)| session.project_id == project.id)
                    .collect();
                if sessions.is_empty() {
                    continue;
                }
                let collapsed = self.preferences.agents_collapsed.contains(&project.id);
                let header = appearance::row(
                    ui,
                    &project.name,
                    if collapsed {
                        "ChevronRight"
                    } else {
                        "ChevronDown"
                    },
                    false,
                    26.0,
                    &sessions.len().to_string(),
                    appearance::color(&self.theme.text),
                )
                .on_hover_text(format!(
                    "{}{}{}",
                    project.path.display(),
                    if self.managed_worktree(&project.id).is_some() {
                        "\nManaged Git worktree"
                    } else {
                        ""
                    },
                    "\nVerified live agents"
                ));
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("live-project:{}", project.id),
                    header.rect,
                );
                if header.clicked() {
                    if collapsed {
                        self.preferences.agents_collapsed.remove(&project.id);
                    } else {
                        self.preferences.agents_collapsed.insert(project.id.clone());
                    }
                }
                if !collapsed {
                    ui.indent(("live-project", &project.id), |ui| {
                        for (session, _) in sessions {
                            self.live_agent_row(ui, session, "live-row");
                        }
                    });
                }
                ui.add_space(4.0);
            }
            if !unverified.is_empty() {
                let collapsed = self.preferences.agents_collapsed.contains("unverified");
                let header = appearance::row(
                    ui,
                    "Presence unverified",
                    if collapsed {
                        "ChevronRight"
                    } else {
                        "ChevronDown"
                    },
                    false,
                    26.0,
                    &unverified.len().to_string(),
                    appearance::color(&self.theme.status_waiting),
                )
                .on_hover_text(
                    "Hook-only or unavailable-owner sessions.\nLast reported hook state; no verified live process.",
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "live-unverified", header.rect);
                if header.clicked() {
                    if collapsed {
                        self.preferences.agents_collapsed.remove("unverified");
                    } else {
                        self.preferences.agents_collapsed.insert("unverified".into());
                    }
                }
                if !collapsed {
                    ui.indent("live-unverified", |ui| {
                        for session in &unverified {
                            self.live_agent_row(ui, session, "unverified-row");
                        }
                    });
                }
            }
        });
    }

    fn live_agent_row(&mut self, ui: &mut egui::Ui, session: &Session, target: &str) {
        let presented = self.present_session(&session.id);
        let mut subtitle = match (&presented.brand_label, presented.status_label.as_str()) {
            (Some(brand), status) => format!("{brand} · {status}"),
            (None, status) => status.to_string(),
        };
        if !presented.verified {
            subtitle.push_str(" · unverified");
        }
        let unread = if presented.unread > 0 {
            format!("{} unread", presented.unread)
        } else {
            String::new()
        };
        let response = appearance::session_row_spec(
            ui,
            appearance::SessionRowSpec {
                label: &session.label,
                icon: match presented.lifecycle {
                    Some(_) => presented.status_icon,
                    None => "Terminal",
                },
                selected: self.active_session.as_ref() == Some(&session.id),
                trailing: &unread,
                tint: appearance::color(&self.theme.secondary),
                icon_tint: presented
                    .lifecycle
                    .map(|state| state_color(state, &self.theme)),
                spin: presented.spin,
                subtitle: Some(&subtitle),
                brand: presented.brand_icon,
            },
        )
        .on_hover_ui(|ui| {
            ui.label(presented.diagnostics(now()));
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("{target}:{}", session.id), response.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = target;
        if response.clicked() {
            self.go_session(&session.id);
        }
    }

    fn agents_filter_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let current = self.preferences.agents_filter.clone();
        let label = if current.is_empty() {
            "All agents".to_string()
        } else {
            terminator_core::agents::display_name(&current).to_string()
        };
        let menu = appearance::menu_button(ui, &label, |ui| {
            let mut kinds: Vec<String> = self
                .state
                .agents
                .iter()
                .map(|a| a.kind.clone())
                .chain(
                    self.state
                        .presence
                        .iter()
                        .flat_map(|p| p.agents.iter().map(|a| a.kind.clone())),
                )
                .collect();
            kinds.sort();
            kinds.dedup();
            for kind in std::iter::once(String::new()).chain(kinds) {
                let name = if kind.is_empty() {
                    "All agents".to_string()
                } else {
                    terminator_core::agents::display_name(&kind).to_string()
                };
                let check = if current == kind { "✓" } else { "" };
                let response = appearance::menu_item(ui, &name, "Search", check);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!(
                        "agents-filter:{}",
                        if kind.is_empty() { "all" } else { &kind }
                    ),
                    response.rect,
                );
                if response.clicked() {
                    self.preferences.agents_filter = kind;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Filter by agent");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "agents-filter", menu.rect);
        let _ = menu;
    }
    fn agents_empty(&mut self, ui: &mut egui::Ui) {
        if self.hook_status.is_empty() {
            ui.weak("Checking agent hooks…");
            return;
        }
        if self.state.agents.is_empty() && !self.hook_status.values().any(|installed| *installed) {
            ui.weak("Agent hooks are not configured");
            if ui.small_button("Set up hooks").clicked() {
                self.open_settings();
                self.settings_section = SettingsSection::AgentHooks;
            }
            return;
        }
        ui.weak("No pending agent events");
    }
    pub(super) fn waiting_notice_count(&self) -> usize {
        self.pending_notices()
            .iter()
            .filter(|notice| notice_waiting(notice))
            .count()
    }
    /// Single source for the waiting/unread counts rendered as bells in
    /// the left agent bar and the IDE status-bar mirror.
    pub(super) fn attention_counts(&self) -> (usize, usize) {
        let waiting = self.waiting_notice_count();
        let unread = self
            .state
            .notifications
            .iter()
            .filter(|notice| !notice.read && notice_pending(notice, now()))
            .count();
        (waiting, unread)
    }
    /// Pending agent notices as menu-bar items, in inbox order (waiting
    /// first). Titles are single-line and capped so the native menu stays
    /// readable. Same list the Agents inbox renders.
    #[cfg(any(test, target_os = "macos"))]
    pub(super) fn status_menu_items(&self) -> Vec<updater::StatusMenuItem> {
        const MAX_ITEMS: usize = 12;
        const MAX_TITLE: usize = 90;
        self.pending_notices()
            .into_iter()
            .take(MAX_ITEMS)
            .map(|notice| {
                let session = self
                    .state
                    .sessions
                    .iter()
                    .find(|session| session.id == notice.session_id)
                    .map(|session| session.label.as_str())
                    .unwrap_or("Terminal");
                let title = format!("{} — {}: {}", session, notice.state.label(), notice.summary);
                let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                let title = if title.chars().count() > MAX_TITLE {
                    format!("{}…", title.chars().take(MAX_TITLE - 1).collect::<String>())
                } else {
                    title
                };
                updater::StatusMenuItem {
                    id: notice.id,
                    title,
                }
            })
            .collect()
    }
    fn pending_notices(&self) -> Vec<Notification> {
        let selected = self.selected.as_deref();
        let mut notices: Vec<_> = self
            .state
            .notifications
            .iter()
            .filter(|notice| {
                !notice.dismissed
                    && !notice.resolved
                    && notice.snoozed_until <= now()
                    && self.notice_in_scope(notice, selected)
            })
            .cloned()
            .collect();
        notices.sort_by_key(|notice| {
            (
                notice.resolved,
                notice_rank(notice.state),
                Reverse(notice.created),
            )
        });
        notices
    }
    fn notice_in_scope(&self, notice: &Notification, selected: Option<&str>) -> bool {
        self.state.sessions.iter().any(|session| {
            session.id == notice.session_id
                && self
                    .preferences
                    .includes_project(&session.project_id, selected)
        })
    }
    /// One row acts as one unit: navigation needs a single target, while
    /// read/snooze/dismiss apply to every notice folded into the row.
    fn apply_group_action(&mut self, group: &NoticeGroup, action: AttentionAction) {
        match action {
            AttentionAction::Go => {
                if let Some(first) = group.notices.first() {
                    self.apply_notice_action(first.id.clone(), action);
                }
            }
            _ => {
                for notice in &group.notices {
                    self.apply_notice_action(notice.id.clone(), action);
                }
            }
        }
    }
    pub(super) fn apply_notice_action(&mut self, id: String, action: AttentionAction) {
        let mut presentation_changed = false;
        match action {
            AttentionAction::None => {}
            AttentionAction::Go => {
                if let Some(session) = self
                    .state
                    .notifications
                    .iter()
                    .find(|notice| notice.id == id)
                    .map(|notice| notice.session_id.clone())
                {
                    self.go_session(&session);
                }
                self.unread_selected = Some(id.clone());
                self.detail = None;
            }
            AttentionAction::Read => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "read".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    // Read state only: never resolve, dismiss, or touch lifecycle.
                    notice.read = true;
                    presentation_changed = true;
                }
                self.unread_selected = Some(id);
            }
            AttentionAction::Snooze => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "snooze".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    notice.snoozed_until = now() + 600;
                    presentation_changed = true;
                }
                if self.detail.as_ref() == Some(&id) {
                    self.detail = None;
                }
            }
            AttentionAction::Dismiss => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "dismiss".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    notice.dismissed = true;
                    presentation_changed = true;
                }
                if self.unread_selected.as_deref() == Some(id.as_str()) {
                    self.unread_selected = None;
                }
                if self.detail.as_ref() == Some(&id) {
                    self.detail = None;
                }
            }
        }
        if presentation_changed {
            self.reconcile_presentations();
        }
    }
    fn info_panel(&mut self, ui: &mut egui::Ui) {
        let session = self.context_session();
        let live = session.is_some_and(|session| session.lifecycle.live() && session.pid.is_some());
        let pid = session.and_then(|session| session.pid);
        let created = session.map(|session| session.created).unwrap_or(0);
        let sample = self
            .resources
            .as_ref()
            .filter(|sample| sample.pid == pid && sample.started == created);
        let input = session_info::Input {
            label: session.map(|session| session.label.clone()),
            cwd: session.map(|session| crate::services::compact_path(&session.cwd)),
            cwd_full: session.map(|session| session.cwd.display().to_string()),
            branch: session.and_then(|session| self.branch_at(&session.cwd)),
            started: session_info::started_label(created, now(), live),
            unconfirmed: session.is_some_and(|session| !session.cwd_confirmed),
            session_cpu: sample
                .and_then(|sample| sample.session)
                .map(|stats| stats.cpu),
            session_memory: sample
                .and_then(|sample| sample.session)
                .map(|stats| stats.memory),
            system: self.resources.as_ref().map(|sample| sample.system.clone()),
        };
        let mut toggles = session_info::Toggles {
            process_open: self.preferences.info_process_open,
            resources_open: self.preferences.info_resources_open,
            show_system: self.preferences.info_show_system,
        };
        let model = session_info::model(&input);
        session_info::show(ui, &model, &mut toggles, &self.theme);
        self.preferences.info_process_open = toggles.process_open;
        self.preferences.info_resources_open = toggles.resources_open;
        self.preferences.info_show_system = toggles.show_system;
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        if self.preferences.tool == SidebarTool::Info {
            self.info_panel(ui);
            return;
        }
        if self.preferences.tool == SidebarTool::History {
            let ended: Vec<_> = self
                .state
                .sessions
                .iter()
                .filter(|s| !s.lifecycle.live() && self.state.session_has_resume(&s.id))
                .cloned()
                .collect();
            let groups = sort_history(HistoryInput {
                projects: self.state.projects.clone(),
                sessions: &ended,
                agents: &self.state.agents,
                notifications: &self.state.notifications,
                terminal_notices: &self.state.terminal_notices,
                sort: self.preferences.history_sort,
                filter: &self.preferences.history_filter.clone(),
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let width = (ui.available_width() - 160.0).max(60.0);
                let response = ui.add_sized(
                    egui::vec2(width, 22.0),
                    egui::TextEdit::singleline(&mut self.preferences.history_filter)
                        .id(egui::Id::new("history-filter"))
                        .hint_text("Filter by name"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "history-filter", response.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &response;
                if !self.preferences.history_filter.is_empty()
                    && ui.small_button("✕").on_hover_text("Clear filter").clicked()
                {
                    self.preferences.history_filter.clear();
                }
                let all_expanded = !groups.is_empty()
                    && groups.iter().all(|group| {
                        self.preferences
                            .history_expanded
                            .get(&group.project.id)
                            .copied()
                            .unwrap_or(true)
                    });
                let toggle = ui
                    .add_enabled(
                        !groups.is_empty(),
                        egui::Button::new(if all_expanded {
                            "Collapse all"
                        } else {
                            "Expand all"
                        }),
                    )
                    .on_hover_text("Collapse or expand all History projects");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "history-toggle-all", toggle.rect);
                if toggle.clicked() {
                    for group in &groups {
                        self.preferences
                            .history_expanded
                            .insert(group.project.id.clone(), !all_expanded);
                    }
                }
                self.history_sort_menu(ui);
            });
            ui.add_space(4.0);
            appearance::sidebar_scroll("global-history").show(ui, |ui| {
                if ended.is_empty() {
                    ui.weak("No resumable sessions.");
                } else if groups.is_empty() {
                    ui.weak("No matching sessions.");
                }
                for group in groups {
                    let expanded = *self
                        .preferences
                        .history_expanded
                        .entry(group.project.id.clone())
                        .or_insert(true);
                    let header = appearance::row(
                        ui,
                        &group.project.name,
                        if expanded {
                            "ChevronDown"
                        } else {
                            "ChevronRight"
                        },
                        false,
                        26.0,
                        &group.sessions.len().to_string(),
                        appearance::color(&self.theme.text),
                    )
                    .on_hover_text(group.project.path.display().to_string());
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("history-project:{}", group.project.id),
                        header.rect,
                    );
                    if header.clicked() {
                        self.preferences
                            .history_expanded
                            .insert(group.project.id.clone(), !expanded);
                    }
                    if expanded {
                        ui.indent(("history-project", &group.project.id), |ui| {
                            for session in &group.sessions {
                                self.session_row(ui, session);
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
            });
            return;
        }
        if self.preferences.tool == SidebarTool::Agents {
            self.agents_view(ui);
            return;
        }
        if self.preferences.tool == SidebarTool::Explorer {
            self.pending_delete_bar(ui);
            if let Some(cwd) = self.cwd() {
                self.explorer_toolbar(ui, &cwd);
                let contents =
                    self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
                if contents {
                    appearance::sidebar_scroll("files-search")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.explorer_results(ui, &cwd));
                } else {
                    appearance::sidebar_scroll("files")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.tree(ui, &cwd, 0));
                }
            }
            return;
        }
        if self.context_session().is_some_and(|s| !s.cwd_confirmed) {
            ui.label(RichText::new("Last known directory").small().weak());
        }
        ui.separator();
        if self.watch_fallback {
            ui.weak("Filesystem watch unavailable; refreshing every 3 seconds");
        }
        self.pending_delete_bar(ui);
        if let Some(context) = self.context.clone() {
            self.ensure_git_lists(context.root.clone());
            let open_shortcut = self.shortcut_label("open_file");
            let mut draft = std::mem::take(&mut self.git_commit);
            let outcome = git_panel(
                ui,
                &mut GitPanelInput {
                    context: &context,
                    review_mode: self.state.settings.review_mode,
                    neovim_review: self
                        .state
                        .capabilities
                        .iter()
                        .any(|c| c == NVIM_REVIEW_CAPABILITY),
                    theme: &self.theme,
                    history: self.git_history,
                    view_list: self.preferences.git_view_list,
                    commits: &self.git_log,
                    branches: &self.git_branches,
                    commit_draft: &mut draft,
                    collapse_generation: self.git_collapse,
                    open_shortcut: &open_shortcut,
                    compare: self
                        .git_compare
                        .as_ref()
                        .filter(|_| self.git_compare_root == context.root),
                    base_ref: self.git_base_ref.as_deref(),
                },
            );
            self.git_commit = draft;
            self.perform_git_outcome(ui, outcome);
        } else {
            ui.weak("Select a terminal to inspect its context.");
        }
    }
}

pub fn git_color(theme: &AppearanceConfig, status: char) -> Color32 {
    appearance::color(match status {
        'A' => &theme.git_added,
        'M' => &theme.git_modified,
        'D' | '!' => &theme.git_deleted,
        'R' | 'C' | 'U' => &theme.git_untracked,
        _ => &theme.secondary,
    })
}

/// View data for the Git panel. `App` passes a snapshot of its state; the
/// panel paints it and returns actions without touching `App` or `Services`.
pub struct GitPanelInput<'a> {
    pub context: &'a ContextData,
    pub review_mode: ReviewMode,
    pub neovim_review: bool,
    pub theme: &'a AppearanceConfig,
    pub history: bool,
    pub view_list: bool,
    pub commits: &'a [(String, String)],
    pub branches: &'a [String],
    pub commit_draft: &'a mut String,
    pub collapse_generation: u64,
    pub open_shortcut: &'a str,
    /// Branch comparison for the current root, when available.
    pub compare: Option<&'a workspace_ops::CompareData>,
    /// User-picked base ref; `None` means the upstream branch is used.
    pub base_ref: Option<&'a str>,
}

/// A concrete file action with its target, ready for `App` to perform.
pub struct GitFileAction {
    pub action: FileAction,
    pub path: PathBuf,
}

/// What the Git panel asks `App` to do after painting.
#[derive(Default)]
pub struct GitPanelOutcome {
    /// Row clicks mapped to concrete actions. `App` dedups rapid repeats.
    pub clicked: Vec<GitFileAction>,
    /// Context-menu picks, performed directly.
    pub menu: Vec<GitFileAction>,
    /// The error-state Retry button was pressed: re-queue a context refresh.
    pub refresh: bool,
    /// User pressed the View log button.
    pub view_log: bool,
    pub history: Option<bool>,
    pub collapse: bool,
    pub commit: bool,
    pub switch: Option<String>,
    pub stage: Vec<PathBuf>,
    pub unstage: Vec<PathBuf>,
    pub discard: Vec<(PathBuf, bool)>,
    pub delete: Vec<PathBuf>,
    pub copy: Vec<String>,
    /// Stage every working-tree and untracked change.
    pub stage_all: bool,
    /// Switch between grouped sections and one flat list.
    pub toggle_list: bool,
    /// Store a user-picked base ref; `clear_base` restores the upstream.
    pub set_base: Option<String>,
    pub clear_base: bool,
    pub refresh_compare: bool,
}

/// Map a Git row click to the concrete action `App` performs. Conflicts open
/// as files; a deleted row with no staged side is a no-op; otherwise the diff
/// viewer preference picks native vs Neovim (Neovim only when the daemon
/// advertises it).
pub fn git_click_action(
    deleted: bool,
    staged: Option<bool>,
    clicked: bool,
    review_mode: ReviewMode,
    neovim_review: bool,
) -> Option<FileAction> {
    if !clicked {
        return None;
    }
    let Some(staged) = staged else {
        return if deleted {
            None
        } else {
            Some(FileAction::Open)
        };
    };
    if review_mode != ReviewMode::Neovim || !neovim_review {
        return Some(if staged {
            FileAction::NativeStagedDiff
        } else {
            FileAction::NativeWorkingDiff
        });
    }
    Some(if staged {
        FileAction::StagedDiff
    } else {
        FileAction::WorkingDiff
    })
}

pub fn git_panel(ui: &mut egui::Ui, input: &mut GitPanelInput) -> GitPanelOutcome {
    let mut outcome = GitPanelOutcome::default();
    let branch_name = input.context.branch.clone();
    let root = input.context.root.clone();
    let changes = input.context.changes.clone();
    let error = input.context.error.clone();
    let stats = input.context.stats.clone();
    if root.is_none() {
        ui.weak("Not a Git repository");
    } else {
        git_toolbar(ui, input, &branch_name, &mut outcome);
        if !input.history
            && let Some(compare) = input.compare
        {
            git_compare_row(ui, input.theme, &branch_name, compare, input.base_ref);
        }
        let staged = changes
            .iter()
            .any(|change| change.in_group(terminator_git::GitGroup::Staged));
        if !input.history {
            ui.add(
                egui::TextEdit::multiline(input.commit_draft)
                    .hint_text("Message")
                    .desired_width(ui.available_width())
                    .desired_rows(3),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let stage_all = ui
                    .add_enabled(!changes.is_empty(), egui::Button::new("Stage All"))
                    .on_hover_text("Stage every change");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-stage-all", stage_all.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &stage_all;
                if stage_all.clicked() {
                    outcome.stage_all = true;
                }
                let commit = ui.add_enabled(
                    staged && !input.commit_draft.trim().is_empty(),
                    egui::Button::new("Commit"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-commit", commit.rect);
                if commit.clicked() {
                    outcome.commit = true;
                }
            });
        }
        if input.history {
            appearance::sidebar_scroll("git-log").show(ui, |ui| {
                if input.commits.is_empty() {
                    ui.weak("No commits");
                }
                for (hash, subject) in input.commits {
                    let row = appearance::row(
                        ui,
                        subject,
                        "GitBranch",
                        false,
                        22.0,
                        hash,
                        appearance::color(&input.theme.secondary),
                    );
                    if row.clicked() {
                        outcome.view_log = true;
                    }
                }
            });
            return outcome;
        }
        if changes.is_empty() {
            ui.weak("Working tree clean");
        }
        appearance::sidebar_scroll("git").show(ui, |ui| {
            let row_ctx = GitRowCtx {
                theme: input.theme,
                root: root.as_deref(),
                review_mode: input.review_mode,
                neovim_review: input.neovim_review,
                open_shortcut: input.open_shortcut,
                stats: &stats,
            };
            if input.view_list {
                let mut any = false;
                for group in terminator_git::GitGroup::ALL {
                    for change in changes.iter().filter(|change| change.in_group(group)) {
                        any = true;
                        let label = workspace_ops::relative_display(
                            root.as_deref().unwrap_or(&change.path),
                            &change.path,
                        );
                        git_change_row(ui, &row_ctx, &mut outcome, change, group, &label);
                    }
                }
                if !any {
                    ui.weak("Working tree clean");
                }
            } else {
                for group in terminator_git::GitGroup::ALL {
                    let entries: Vec<_> = changes
                        .iter()
                        .filter(|change| change.in_group(group))
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    git_group_section(
                        ui,
                        &row_ctx,
                        &mut outcome,
                        root.as_deref(),
                        group,
                        &entries,
                        input.collapse_generation,
                    );
                }
                if let Some(compare) = input.compare
                    && !compare.files.is_empty()
                {
                    git_committed_section(
                        ui,
                        &row_ctx,
                        &mut outcome,
                        root.as_deref(),
                        &compare.files,
                        input.collapse_generation,
                    );
                }
            }
        });
    }
    if let Some(e) = &error {
        ui.horizontal(|ui| {
            ui.colored_label(appearance::color(&input.theme.status_failed), e);
            if ui.small_button("Retry").clicked() {
                outcome.refresh = true;
            }
        });
    }
    outcome
}

/// Shared view references for a Git file row.
struct GitRowCtx<'a> {
    theme: &'a AppearanceConfig,
    root: Option<&'a Path>,
    review_mode: ReviewMode,
    neovim_review: bool,
    open_shortcut: &'a str,
    stats: &'a std::collections::HashMap<PathBuf, (u32, u32)>,
}

fn git_group_label(group: terminator_git::GitGroup) -> &'static str {
    use terminator_git::GitGroup;
    match group {
        GitGroup::Conflicts => "CONFLICTS",
        GitGroup::Staged => "STAGED",
        GitGroup::Changes => "CHANGES",
        GitGroup::Untracked => "UNTRACKED FILES",
    }
}

/// Icon toolbar: collapse, Changes/History, branch switcher, refresh, log, and
/// the overflow menu (View as list / Change Base Ref / Refresh branch compare).
fn git_toolbar(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    branch_name: &str,
    outcome: &mut GitPanelOutcome,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        let collapse = appearance::sidebar_action(ui, "ChevronsDownUp", "Collapse all");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-collapse", collapse.rect);
        if collapse.clicked() {
            outcome.collapse = true;
        }
        let changes = appearance::selectable_icon(ui, "FileDiff", "Changes", !input.history);
        let history = appearance::selectable_icon(ui, "History", "History", input.history);
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "git-changes", changes.rect);
            diagnostics::record(ui.ctx(), "git-history", history.rect);
        }
        if changes.clicked() {
            outcome.history = Some(false);
        }
        if history.clicked() {
            outcome.history = Some(true);
        }
        let branch = appearance::text_menu_button(ui, branch_name, |ui| {
            if input.branches.is_empty() {
                ui.weak("No branches");
            }
            for name in input.branches {
                let mark = if name == branch_name { "✓" } else { "" };
                if appearance::menu_item(ui, name, "GitBranch", mark).clicked() {
                    outcome.switch = Some(name.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Switch branch");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-branch", branch.rect);
        let _ = branch;
        let refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-refresh", refresh.rect);
        if refresh.clicked() {
            outcome.refresh = true;
        }
        let log = appearance::sidebar_action(ui, "ListTree", "View commit log");
        if log.clicked() {
            outcome.view_log = true;
        }
        let more = appearance::compact_menu_button(ui, "…", |ui| {
            let mark = if input.view_list { "✓" } else { "" };
            if appearance::menu_item(ui, "View as list", "List", mark).clicked() {
                outcome.toggle_list = true;
                ui.close();
            }
            let base = appearance::menu_button(ui, "Change Base Ref…", |ui| {
                let auto = if input.base_ref.is_none() { "✓" } else { "" };
                if appearance::menu_item(ui, "Automatic (upstream)", "GitCompareArrows", auto)
                    .clicked()
                {
                    outcome.clear_base = true;
                    ui.close();
                }
                ui.separator();
                if input.branches.is_empty() {
                    ui.weak("No branches");
                }
                for name in input.branches {
                    let mark = if input.base_ref == Some(name.as_str()) {
                        "✓"
                    } else {
                        ""
                    };
                    if appearance::menu_item(ui, name, "GitBranch", mark).clicked() {
                        outcome.set_base = Some(name.clone());
                        ui.close();
                    }
                }
            });
            let _ = base;
            ui.separator();
            if appearance::menu_item(ui, "Refresh branch compare", "RefreshCw", "").clicked() {
                outcome.refresh_compare = true;
                ui.close();
            }
        })
        .response
        .on_hover_text("More Git actions");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-more", more.rect);
        let _ = more;
    });
}

/// `branch → base` with ahead/behind counts, the Orca branch compare line.
fn git_compare_row(
    ui: &mut egui::Ui,
    theme: &AppearanceConfig,
    branch: &str,
    compare: &workspace_ops::CompareData,
    base_ref: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::Label::new(RichText::new(branch).monospace().strong()).truncate());
        let target = base_ref
            .or(compare.base.as_deref())
            .or(compare.upstream.as_deref());
        if let Some(target) = target {
            ui.add(
                egui::Label::new(RichText::new(format!("→ {target}")).monospace().weak())
                    .truncate(),
            )
            .on_hover_text("Base ref for the branch comparison");
        }
        if compare.ahead > 0 {
            ui.label(
                RichText::new(format!("↑{}", compare.ahead))
                    .color(appearance::color(&theme.git_added)),
            )
            .on_hover_text("Commits ahead of the base ref");
        }
        if compare.behind > 0 {
            ui.label(RichText::new(format!("↓{}", compare.behind)).weak())
                .on_hover_text("Commits behind the base ref");
        }
    });
}

/// One changed file: diffstat, status letter, click-to-diff, and context menu.
fn git_change_row(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    change: &terminator_git::Change,
    group: terminator_git::GitGroup,
    label: &str,
) {
    // Keep layout height without constructing thousands of off-screen widgets.
    if skip_clipped_git_row(ui) {
        return;
    }
    let letter = change.letter(group);
    let trailing = match ctx.stats.get(&change.path).copied() {
        Some((added, deleted)) if deleted > 0 => format!("+{added} -{deleted} {letter}"),
        Some((added, _)) => format!("+{added} {letter}"),
        None => letter.to_string(),
    };
    let response = appearance::file_row(
        ui,
        label,
        icons::file_icon(&change.path),
        false,
        GIT_TREE_ROW_HEIGHT,
        &trailing,
        git_color(ctx.theme, letter),
    )
    .on_hover_text(format!(
        "{}\n{} ({})",
        change.path.display(),
        terminator_git::status_description(letter),
        group.label()
    ));
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!(
            "git-file-{}",
            change
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ),
        response.rect,
    );
    let staged = (!change.conflict()).then_some(group == terminator_git::GitGroup::Staged);
    if let Some(action) = git_click_action(
        letter == 'D',
        staged,
        response.clicked() || response.double_clicked(),
        ctx.review_mode,
        ctx.neovim_review,
    ) {
        outcome.clicked.push(GitFileAction {
            action,
            path: change.path.clone(),
        });
    }
    appearance::context_menu(&response, |ui| {
        let open_changes = git_click_action(
            letter == 'D',
            staged,
            true,
            ctx.review_mode,
            ctx.neovim_review,
        );
        if let Some(action) = open_changes
            && appearance::menu_item(ui, "Open changes", "FileDiff", "").clicked()
        {
            outcome.menu.push(GitFileAction {
                action,
                path: change.path.clone(),
            });
            ui.close();
        }
        if appearance::menu_item(ui, "Open file", "FileCode", ctx.open_shortcut).clicked() {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: change.path.clone(),
            });
            ui.close();
        }
        ui.separator();
        if group == terminator_git::GitGroup::Staged {
            if appearance::menu_item(ui, "Unstage", "ArrowLeft", "").clicked() {
                outcome.unstage.push(change.path.clone());
                ui.close();
            }
        } else if appearance::menu_item(ui, "Stage", "Plus", "").clicked() {
            outcome.stage.push(change.path.clone());
            ui.close();
        }
        if appearance::menu_item(ui, "Discard changes", "Eraser", "").clicked() {
            outcome.discard.push((
                change.path.clone(),
                group == terminator_git::GitGroup::Untracked,
            ));
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
            outcome.copy.push(change.path.display().to_string());
            ui.close();
        }
        if let Some(root) = ctx.root
            && appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked()
        {
            outcome
                .copy
                .push(workspace_ops::relative_display(root, &change.path));
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Delete file", "X", "").clicked() {
            outcome.delete.push(change.path.clone());
            ui.close();
        }
    });
}

const GIT_SECTION_LIMIT: usize = 8;
/// Per-level indent and row height for the changed-file tree. Kept tight so a
/// deep path stays legible in the narrow sidebar.
const GIT_TREE_INDENT: f32 = 12.0;
const GIT_TREE_ROW_HEIGHT: f32 = 20.0;

/// Orca-style section header with per-section stage/unstage all. The body is a
/// directory tree (JetBrains-style), collapsible per folder.
#[allow(clippy::too_many_arguments)]
fn git_group_section(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    root: Option<&Path>,
    group: terminator_git::GitGroup,
    entries: &[&terminator_git::Change],
    collapse_generation: u64,
) {
    let label = git_group_label(group);
    let total = entries.len();
    let id = ui.make_persistent_id(("git-section", root, label, collapse_generation));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        collapse_generation == 0,
    );
    let _ = state
        .show_header(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(format!("{label} {total}")).small().strong())
                    .selectable(false),
            );
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match group {
                    terminator_git::GitGroup::Staged => {
                        let action = appearance::sidebar_action(ui, "ArrowLeft", "Unstage all");
                        if action.clicked() {
                            for change in entries {
                                outcome.unstage.push(change.path.clone());
                            }
                        }
                    }
                    terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                        let action = appearance::sidebar_action(ui, "Plus", "Stage all");
                        if action.clicked() {
                            for change in entries {
                                outcome.stage.push(change.path.clone());
                            }
                        }
                    }
                    terminator_git::GitGroup::Conflicts => {}
                },
            );
        })
        .body(|ui| {
            let tree = build_change_tree(entries, root);
            ui.spacing_mut().indent = GIT_TREE_INDENT;
            ui.spacing_mut().item_spacing.y = 0.0;
            git_change_tree(ui, ctx, outcome, &tree, "", group);
        });
}

/// Directory tree of changed files, built once per section paint.
#[derive(Default)]
struct ChangeTree<'a> {
    dirs: std::collections::BTreeMap<String, ChangeTree<'a>>,
    files: Vec<&'a terminator_git::Change>,
}

impl<'a> ChangeTree<'a> {
    fn count_files(&self) -> usize {
        self.files.len()
            + self
                .dirs
                .values()
                .map(ChangeTree::count_files)
                .sum::<usize>()
    }

    fn collect(&self, out: &mut Vec<&'a terminator_git::Change>) {
        out.extend(self.files.iter().copied());
        for dir in self.dirs.values() {
            dir.collect(out);
        }
    }
}

fn build_change_tree<'a>(
    entries: &[&'a terminator_git::Change],
    root: Option<&Path>,
) -> ChangeTree<'a> {
    fn insert<'a>(node: &mut ChangeTree<'a>, dirs: &[String], change: &'a terminator_git::Change) {
        match dirs.split_first() {
            None => node.files.push(change),
            Some((dir, rest)) => insert(node.dirs.entry(dir.clone()).or_default(), rest, change),
        }
    }
    let mut tree = ChangeTree::default();
    for change in entries {
        let relative = root
            .and_then(|root| change.path.strip_prefix(root).ok())
            .unwrap_or(&change.path);
        let components: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if components.is_empty() {
            continue;
        }
        insert(&mut tree, &components[..components.len() - 1], change);
    }
    tree
}

fn git_change_tree(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    node: &ChangeTree<'_>,
    prefix: &str,
    group: terminator_git::GitGroup,
) {
    for (name, child) in &node.dirs {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let id = ui.make_persistent_id(("git-tree", key.clone(), group.label()));
        let count = child.count_files();
        let mut folder_response = None;
        let mut header =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let trailing = count.to_string();
                    folder_response = Some(appearance::file_row(
                        ui,
                        name,
                        "Folder",
                        false,
                        GIT_TREE_ROW_HEIGHT,
                        &trailing,
                        appearance::color(&ctx.theme.secondary),
                    ));
                });
        if folder_response
            .as_ref()
            .is_some_and(|response| response.clicked())
        {
            header.toggle();
        }
        let _ = header.body(|ui| git_change_tree(ui, ctx, outcome, child, &key, group));
        if let Some(response) = folder_response {
            let response = response.on_hover_text(format!("{key} ({count} changed)"));
            appearance::context_menu(&response, |ui| match group {
                terminator_git::GitGroup::Staged => {
                    if appearance::menu_item(ui, "Unstage folder", "ArrowLeft", "").clicked() {
                        let mut files = Vec::new();
                        child.collect(&mut files);
                        for change in files {
                            outcome.unstage.push(change.path.clone());
                        }
                        ui.close();
                    }
                }
                terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                    if appearance::menu_item(ui, "Stage folder", "Plus", "").clicked() {
                        let mut files = Vec::new();
                        child.collect(&mut files);
                        for change in files {
                            outcome.stage.push(change.path.clone());
                        }
                        ui.close();
                    }
                }
                terminator_git::GitGroup::Conflicts => {}
            });
        }
    }
    for change in &node.files {
        let label = change
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        git_change_row(ui, ctx, outcome, change, group, &label);
    }
}

/// Files changed since the base ref, grouped under "COMMITTED ON BRANCH".
fn git_committed_section(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    root: Option<&Path>,
    files: &[workspace_ops::CommittedFile],
    collapse_generation: u64,
) {
    let total = files.len();
    let id = ui.make_persistent_id(("git-committed", root, total, collapse_generation));
    let view_all_id =
        ui.make_persistent_id(("git-committed-all", root, total, collapse_generation));
    let mut expanded = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(view_all_id).unwrap_or(false));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        collapse_generation == 0,
    );
    let _ = state
        .show_header(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("COMMITTED ON BRANCH {total}"))
                        .small()
                        .strong(),
                )
                .selectable(false),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if total > GIT_SECTION_LIMIT {
                    let text = if expanded { "Show less" } else { "View all" };
                    if ui.small_button(text).clicked() {
                        expanded = !expanded;
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(view_all_id, expanded));
                    }
                }
            });
        })
        .body(|ui| {
            let shown = if expanded {
                total
            } else {
                total.min(GIT_SECTION_LIMIT)
            };
            for file in files.iter().take(shown) {
                git_committed_row(ui, ctx, outcome, file);
            }
        });
}

/// One file changed since the base ref. It has no index/worktree side, so the
/// only useful actions are open and copy.
fn git_committed_row(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    file: &workspace_ops::CommittedFile,
) {
    if skip_clipped_git_row(ui) {
        return;
    }
    let name = file
        .path
        .strip_prefix(ctx.root.unwrap_or(&file.path))
        .unwrap_or(&file.path)
        .display()
        .to_string();
    let trailing = if file.deleted > 0 {
        format!("+{} -{} {}", file.added, file.deleted, file.letter)
    } else {
        format!("+{} {}", file.added, file.letter)
    };
    let response = appearance::file_row(
        ui,
        &name,
        icons::file_icon(&file.path),
        false,
        24.0,
        &trailing,
        git_color(ctx.theme, file.letter),
    )
    .on_hover_text(file.path.display().to_string());
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!(
            "git-committed-{}",
            file.path.file_name().unwrap_or_default().to_string_lossy()
        ),
        response.rect,
    );
    if response.clicked() || response.double_clicked() {
        outcome.menu.push(GitFileAction {
            action: FileAction::Open,
            path: file.path.clone(),
        });
    }
    appearance::context_menu(&response, |ui| {
        if appearance::menu_item(ui, "Open file", "FileCode", ctx.open_shortcut).clicked() {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: file.path.clone(),
            });
            ui.close();
        }
        if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
            outcome.copy.push(file.path.display().to_string());
            ui.close();
        }
    });
}

/// Small text toggle used by the Contents search bar (`Aa`, whole word, `.*`).
fn explorer_toggle(
    ui: &mut egui::Ui,
    text: &str,
    tip: &str,
    selected: bool,
    target: &str,
) -> egui::Response {
    let response = ui
        .add_sized(
            [22.0, 22.0],
            egui::Button::new(RichText::new(text).small()).frame(false),
        )
        .on_hover_text(tip);
    if selected || response.hovered() {
        ui.painter().rect_filled(
            response.rect,
            4,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tip)
    });
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), target, response.rect);
    let _ = target;
    response
}

/// Names | Contents segmented control for the Explorer search bar.
fn explorer_segmented(ui: &mut egui::Ui, mode: &mut ExplorerSearchMode) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
    let base_id = response.id;
    ui.painter()
        .rect_filled(rect, 6, ui.visuals().faint_bg_color);
    let half = rect.width() / 2.0;
    for (index, (label, value, target)) in [
        ("Names", ExplorerSearchMode::Names, "explorer-mode-names"),
        (
            "Contents",
            ExplorerSearchMode::Contents,
            "explorer-mode-contents",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + half * index as f32, rect.top()),
            egui::vec2(half, rect.height()),
        );
        let response = ui
            .interact(cell, base_id.with(label), egui::Sense::click())
            .on_hover_text(label);
        let selected = *mode == value;
        if selected {
            ui.painter()
                .rect_filled(cell.shrink(3.0), 5, ui.visuals().widgets.active.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(cell.shrink(3.0), 5, ui.visuals().widgets.hovered.bg_fill);
        }
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            if selected {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), target, cell);
        let _ = target;
        if response.clicked() {
            *mode = value;
        }
    }
}

/// What an explorer file row asks `App` to do after painting.
pub enum ExplorerLocal {
    Reveal,
    CopyRelative,
    Rename,
    Duplicate,
    Delete,
}

#[derive(Default)]
pub struct ExplorerRowOutcome {
    pub clicked: Option<FileAction>,
    pub menu: Option<FileAction>,
    pub local: Option<ExplorerLocal>,
}

#[allow(clippy::too_many_arguments)]
pub fn explorer_file_row(
    ui: &mut egui::Ui,
    path: &Path,
    label: &str,
    status: char,
    ignored: bool,
    theme: &AppearanceConfig,
    open_shortcut: &str,
    split_shortcut: &str,
) -> ExplorerRowOutcome {
    let mut outcome = ExplorerRowOutcome::default();
    let color = if ignored {
        appearance::color(&theme.git_ignored)
    } else {
        git_color(theme, status)
    };
    let r = appearance::file_row(
        ui,
        label,
        icons::file_icon(path),
        false,
        24.0,
        &status.to_string(),
        color,
    )
    .on_hover_text(format!(
        "{}\n{}",
        path.display(),
        if ignored {
            "Ignored"
        } else {
            terminator_git::status_description(status)
        }
    ));
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), &format!("explorer-file:{label}"), r.rect);
    if r.clicked() || r.double_clicked() {
        outcome.clicked = Some(FileAction::Open);
    }
    appearance::context_menu(&r, |ui| {
        if let Some(action) = file_actions::menu_with(
            ui,
            file_actions::FileMenu {
                file: true,
                browser: file_actions::browser_document(path),
                git: None,
                neovim: false,
            },
            open_shortcut,
            split_shortcut,
        ) {
            outcome.menu = Some(action);
        }
        ui.separator();
        if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "").clicked() {
            outcome.local = Some(ExplorerLocal::Reveal);
            ui.close();
        }
        if appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked() {
            outcome.local = Some(ExplorerLocal::CopyRelative);
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Rename", "Pencil", "").clicked() {
            outcome.local = Some(ExplorerLocal::Rename);
            ui.close();
        }
        if appearance::menu_item(ui, "Duplicate", "Copy", "").clicked() {
            outcome.local = Some(ExplorerLocal::Duplicate);
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Delete", "X", "").clicked() {
            outcome.local = Some(ExplorerLocal::Delete);
            ui.close();
        }
    });
    outcome
}

pub(super) fn state_color(state: AgentState, theme: &AppearanceConfig) -> Color32 {
    appearance::color(match state {
        AgentState::Running => &theme.status_running,
        AgentState::WaitingInput | AgentState::WaitingPermission => &theme.status_waiting,
        AgentState::Failed => &theme.status_failed,
        AgentState::Completed => &theme.accent,
        _ => &theme.secondary,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AttentionAction {
    None,
    Go,
    Snooze,
    Dismiss,
    /// Mark read without resolving, dismissing, or changing lifecycle.
    Read,
}

pub(super) struct AttentionCard<'a> {
    pub theme: &'a AppearanceConfig,
    pub notice: &'a Notification,
    pub session: Option<&'a Session>,
    pub selected: bool,
    pub highlight: bool,
    /// Stable agent brand beside the status glyph, from the shared model.
    pub brand_icon: Option<&'static str>,
    pub brand_label: Option<&'a str>,
    /// Show the mark-read button (Unread view only).
    pub show_read: bool,
    /// Further notices from the same agent run folded into this row.
    pub group_extra: &'a [Notification],
}

fn notice_waiting(notice: &Notification) -> bool {
    !notice.resolved
        && matches!(
            notice.state,
            AgentState::WaitingInput | AgentState::WaitingPermission
        )
}

fn notice_rank(state: AgentState) -> u8 {
    match state {
        AgentState::WaitingInput | AgentState::WaitingPermission => 0,
        AgentState::Failed => 1,
        AgentState::Completed => 2,
        _ => 3,
    }
}

/// One inbox row: every pending notice from a single agent run. Focus
/// dismissal already clears the whole session at once, so the row acts as
/// one unit while the inbox keeps each underlying notice.
struct NoticeGroup {
    notices: Vec<Notification>,
}

/// Groups by agent run, preserving input order; the first notice of each
/// group is its representative. Callers pass urgency/newest-sorted input.
fn group_notices(notices: Vec<Notification>) -> Vec<NoticeGroup> {
    let mut groups: Vec<NoticeGroup> = Vec::new();
    for notice in notices {
        if let Some(group) = groups.iter_mut().find(|group| {
            group.notices.first().is_some_and(|first| {
                first.session_id == notice.session_id && first.invocation_id == notice.invocation_id
            })
        }) {
            group.notices.push(notice);
        } else {
            groups.push(NoticeGroup {
                notices: vec![notice],
            });
        }
    }
    groups
}

fn notice_pending(notice: &Notification, timestamp: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= timestamp
}

fn attention_badge_label(waiting: usize, unread: usize) -> String {
    match (waiting, unread) {
        (0, 0) => String::new(),
        (0, unread) => format!("{unread} unread"),
        (waiting, 0) => format!("{waiting} waiting"),
        (waiting, unread) => format!("{waiting} waiting · {unread} unread"),
    }
}

fn notice_preview(markdown: &str) -> String {
    super::agent_presence::notice_preview(markdown)
}

const ATTENTION_ACTION_SIZE: f32 = 22.0;
const ATTENTION_ACTION_COUNT: f32 = 3.0;
const ATTENTION_ACTION_READ_COUNT: f32 = 4.0;

pub(super) use super::agent_presence::attention_status_icon;

fn attention_status_detail(state: AgentState) -> &'static str {
    match state {
        AgentState::WaitingInput => "Reply in the terminal to continue.",
        AgentState::WaitingPermission => "Approve or deny the agent's request.",
        AgentState::Completed => "This turn finished.",
        AgentState::Failed => "The agent reported an error.",
        AgentState::Running => "The agent is working.",
        AgentState::Stopped => "The agent was stopped.",
        AgentState::Unknown => "No status has been reported yet.",
    }
}

/// Width left for the session label after the status glyph (and optional
/// brand) are already placed. `remaining` is that leftover row width.
fn attention_label_width(remaining: f32, spacing: f32, show_read: bool) -> f32 {
    let actions = if show_read {
        ATTENTION_ACTION_READ_COUNT
    } else {
        ATTENTION_ACTION_COUNT
    };
    (remaining - spacing - ATTENTION_ACTION_SIZE * actions).max(0.0)
}

fn attention_status_glyph(
    ui: &mut egui::Ui,
    state: AgentState,
    theme: &AppearanceConfig,
) -> egui::Response {
    let tint = state_color(state, theme);
    let response = ui
        .add_sized(
            [ATTENTION_ACTION_SIZE, ATTENTION_ACTION_SIZE],
            egui::Button::image(
                egui::Image::new(crate::icons::source(attention_status_icon(state)))
                    .tint(tint)
                    .fit_to_exact_size(egui::vec2(14.0, 14.0)),
            )
            .frame(false),
        )
        .on_hover_ui(|ui| {
            ui.set_max_width(240.0);
            ui.colored_label(tint, state.label());
            ui.label(attention_status_detail(state));
        });
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, state.label()));
    response
}

fn attention_title(
    ui: &mut egui::Ui,
    notice: &Notification,
    group_extra: &[Notification],
    session: Option<&Session>,
    theme: &AppearanceConfig,
    brand: Option<(&'static str, &str)>,
    show_read: bool,
) -> egui::Response {
    let spacing = ui.spacing().item_spacing.x;
    let brand_response = brand.map(|(icon, label)| {
        ui.add_sized(
            [18.0, ATTENTION_ACTION_SIZE],
            egui::Image::new(crate::icons::source(icon))
                .fit_to_exact_size(egui::vec2(14.0, 14.0))
                .sense(egui::Sense::hover()),
        )
        .on_hover_text(label)
    });
    let icon = attention_status_glyph(ui, notice.state, theme);
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!("agent-status:{}", notice.session_id),
        icon.rect,
    );
    let session_label = session.map_or("Unknown session", |session| session.label.as_str());
    let text = if group_extra.is_empty() {
        session_label.to_string()
    } else {
        format!("{session_label} · {} events", group_extra.len() + 1)
    };
    // Measure after the leading icons so a brand glyph cannot push the
    // actions onto a second row.
    let label_width = attention_label_width(ui.available_width(), spacing, show_read);
    let label = ui
        .add_sized(
            [label_width, ATTENTION_ACTION_SIZE],
            egui::Label::new(RichText::new(text).size(12.0))
                .truncate()
                .sense(egui::Sense::click()),
        )
        .on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            if let Some(session) = session {
                ui.weak(session.cwd.display().to_string());
            }
            ui.label(notice_preview(&notice.summary));
            for extra in group_extra.iter().take(8) {
                ui.label(format!(
                    "{}: {}",
                    extra.state.label(),
                    notice_preview(&extra.summary)
                ));
            }
            if group_extra.len() > 8 {
                ui.weak(format!("…and {} more", group_extra.len() - 8));
            }
        });
    match brand_response {
        Some(brand) => brand.union(icon).union(label),
        None => icon.union(label),
    }
}

fn attention_actions(ui: &mut egui::Ui, session_id: &str, show_read: bool) -> AttentionAction {
    let count = if show_read {
        ATTENTION_ACTION_READ_COUNT
    } else {
        ATTENTION_ACTION_COUNT
    };
    let width = ATTENTION_ACTION_SIZE * count;
    ui.push_id(session_id, |ui| {
        ui.horizontal(|ui| {
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size =
                egui::vec2(ATTENTION_ACTION_SIZE, ATTENTION_ACTION_SIZE);
            let mut action = AttentionAction::None;
            let mut buttons = vec![
                ("ArrowRight", "Go to context", "go", AttentionAction::Go),
                (
                    "Moon",
                    "Snooze 10 minutes",
                    "snooze",
                    AttentionAction::Snooze,
                ),
                ("X", "Dismiss", "dismiss", AttentionAction::Dismiss),
            ];
            if show_read {
                buttons.insert(
                    1,
                    (
                        "CircleCheck",
                        "Mark read (keeps agent state)",
                        "read",
                        AttentionAction::Read,
                    ),
                );
            }
            for (icon, tip, name, next) in buttons {
                let response = appearance::sidebar_action(ui, icon, tip);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("agent-{name}:{session_id}"),
                    response.rect,
                );
                #[cfg(not(feature = "test-support"))]
                let _ = name;
                if response.clicked() {
                    action = next;
                }
            }
            action
        })
        .inner
    })
    .inner
}

pub(super) fn attention_card(ui: &mut egui::Ui, input: AttentionCard<'_>) -> AttentionAction {
    let AttentionCard {
        theme,
        notice,
        session,
        selected,
        highlight,
        brand_icon,
        brand_label,
        show_read,
        group_extra,
    } = input;
    let brand = match (brand_icon, brand_label) {
        (Some(icon), Some(label)) => Some((icon, label)),
        (Some(icon), None) => Some((icon, "Agent")),
        _ => None,
    };
    let stroke = if highlight {
        egui::Stroke::new(1.0, appearance::color(&theme.accent))
    } else if selected {
        egui::Stroke::new(1.0, appearance::color(&theme.selection))
    } else {
        egui::Stroke::new(theme.border_width, appearance::color(&theme.border))
    };
    let inner = egui::Frame::group(ui.style())
        .stroke(stroke)
        .corner_radius(6)
        .fill(appearance::color(&theme.window))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
            let action = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                    let header =
                        attention_title(ui, notice, group_extra, session, theme, brand, show_read);
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("agent-row:{}", notice.session_id),
                        header.rect,
                    );
                    let mut action = if header.clicked() {
                        AttentionAction::Go
                    } else {
                        AttentionAction::None
                    };
                    let buttons = attention_actions(ui, &notice.session_id, show_read);
                    if buttons != AttentionAction::None {
                        action = buttons;
                    }
                    action
                })
                .inner;
            if notice.resolved {
                ui.weak("This event has resolved.");
            }
            action
        });
    // Header and action buttons own clicks. A later frame-wide click target
    // sits on top of those buttons in egui and would steal Dismiss/Snooze as Go.
    inner.inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_badge_label_covers_the_count_matrix() {
        assert_eq!(attention_badge_label(0, 0), "");
        assert_eq!(attention_badge_label(0, 2), "2 unread");
        assert_eq!(attention_badge_label(3, 0), "3 waiting");
        assert_eq!(attention_badge_label(3, 2), "3 waiting · 2 unread");
    }

    #[test]
    fn inbox_rows_fold_notices_from_the_same_agent_run() {
        fn notice(id: &str, session: &str, invocation: &str, created: u64) -> Notification {
            Notification {
                id: id.into(),
                session_id: session.into(),
                invocation_id: invocation.into(),
                request_id: None,
                state: AgentState::WaitingInput,
                summary: String::new(),
                details: String::new(),
                created,
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            }
        }
        let groups = group_notices(vec![
            notice("n1", "s", "a", 3),
            notice("n2", "s", "a", 2),
            notice("n3", "s", "b", 1),
            notice("n4", "t", "a", 0),
        ]);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].notices.len(), 2);
        assert_eq!(groups[0].notices[0].id, "n1");
        assert_eq!(groups[0].notices[1].id, "n2");
        assert_eq!(groups[1].notices[0].id, "n3");
        // The same invocation id in another session is a different run.
        assert_eq!(groups[2].notices[0].id, "n4");
        assert!(group_notices(vec![]).is_empty());
    }

    #[test]
    fn git_click_actions_route_by_viewer_preference_and_capability() {
        use FileAction::*;
        assert_eq!(ReviewMode::default(), ReviewMode::Native);
        for (deleted, staged, clicked, mode, advertised, expected) in [
            (false, None, true, ReviewMode::Native, false, Some(Open)),
            (false, None, true, ReviewMode::Neovim, true, Some(Open)),
            (true, None, true, ReviewMode::Native, false, None),
            (
                false,
                Some(false),
                true,
                ReviewMode::Native,
                false,
                Some(NativeWorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Native,
                true,
                Some(NativeStagedDiff),
            ),
            (
                false,
                Some(false),
                true,
                ReviewMode::Neovim,
                false,
                Some(NativeWorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Neovim,
                false,
                Some(NativeStagedDiff),
            ),
            (
                false,
                Some(false),
                true,
                ReviewMode::Neovim,
                true,
                Some(WorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Neovim,
                true,
                Some(StagedDiff),
            ),
            (
                true,
                Some(false),
                true,
                ReviewMode::Neovim,
                true,
                Some(WorkingDiff),
            ),
            (false, Some(false), false, ReviewMode::Neovim, true, None),
        ] {
            assert_eq!(
                git_click_action(deleted, staged, clicked, mode, advertised),
                expected,
                "deleted={deleted} staged={staged:?} clicked={clicked} mode={mode:?} advertised={advertised}"
            );
        }
    }

    #[test]
    fn large_git_sidebar_only_builds_visible_rows() {
        let ctx = egui::Context::default();
        let mut rendered = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_clip_rect(egui::Rect::from_min_size(
                    ui.next_widget_position(),
                    egui::vec2(300.0, 400.0),
                ));
                let start = ui.next_widget_position().y;
                let stride = 24.0 + ui.spacing().item_spacing.y;
                for _ in 0..23_315 {
                    if skip_clipped_git_row(ui) {
                        continue;
                    }
                    rendered += 1;
                    appearance::file_row(
                        ui,
                        "file.rs",
                        icons::file_icon(std::path::Path::new("file.rs")),
                        false,
                        24.0,
                        "M",
                        egui::Color32::WHITE,
                    );
                }
                assert!((ui.next_widget_position().y - start - stride * 23_315.0).abs() < 2.0);
            });
        });
        output.textures_delta.clear();
        assert!(rendered > 0 && rendered < 40, "built {rendered} rows");
    }

    #[test]
    fn message_preview_preserves_words_and_punctuation_without_markdown() {
        assert_eq!(
            notice_preview("**No.** A different user or a `mini-VM` does not help."),
            "No. A different user or a mini-VM does not help."
        );
        assert_eq!(
            notice_preview("**Ready**.\n\n[Open](https://example.com)"),
            "Ready. Open"
        );
    }

    #[test]
    fn resolved_permission_requests_do_not_count_as_waiting() {
        let mut notice = Notification {
            id: "notice".into(),
            session_id: "session".into(),
            invocation_id: "agent".into(),
            request_id: None,
            state: AgentState::WaitingPermission,
            summary: String::new(),
            details: String::new(),
            created: 0,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        };
        assert!(notice_waiting(&notice));
        assert!(notice_pending(&notice, 10));
        notice.snoozed_until = 11;
        assert!(!notice_pending(&notice, 10));
        notice.snoozed_until = 0;
        notice.dismissed = true;
        assert!(!notice_pending(&notice, 10));
        notice.dismissed = false;
        notice.resolved = true;
        assert!(!notice_pending(&notice, 10));
        assert!(!notice_waiting(&notice));
        notice.state = AgentState::WaitingInput;
        assert!(!notice_waiting(&notice));
        notice.resolved = false;
        assert!(notice_waiting(&notice));
    }

    #[test]
    fn attention_status_icon_is_unique_per_state() {
        let states = [
            AgentState::Unknown,
            AgentState::Running,
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Completed,
            AgentState::Failed,
            AgentState::Stopped,
        ];
        let icons: Vec<_> = states.into_iter().map(attention_status_icon).collect();
        let unique: std::collections::HashSet<_> = icons.iter().copied().collect();
        assert_eq!(unique.len(), icons.len());
        for state in states {
            assert!(!attention_status_detail(state).is_empty());
            assert!(!state.label().is_empty());
        }
    }

    #[test]
    fn attention_label_width_leaves_the_action_row_intact() {
        let spacing = 4.0;
        let remaining = 200.0;
        let width = attention_label_width(remaining, spacing, false);
        let used = width + spacing + ATTENTION_ACTION_SIZE * ATTENTION_ACTION_COUNT;
        assert!((used - remaining).abs() < f32::EPSILON);
        assert_eq!(attention_label_width(10.0, spacing, false), 0.0);
        let read = attention_label_width(remaining, spacing, true);
        assert!(read < width);
        let read_used = read + spacing + ATTENTION_ACTION_SIZE * ATTENTION_ACTION_READ_COUNT;
        assert!((read_used - remaining).abs() < f32::EPSILON);
    }

    #[test]
    fn change_tree_nests_directories_and_counts_files() {
        let a = terminator_git::Change {
            path: "/repo/src/a.rs".into(),
            status: " M".into(),
        };
        let b = terminator_git::Change {
            path: "/repo/src/deep/b.rs".into(),
            status: "??".into(),
        };
        let c = terminator_git::Change {
            path: "/repo/root.rs".into(),
            status: " M".into(),
        };
        let entries = vec![&a, &b, &c];
        let tree = build_change_tree(&entries, Some(Path::new("/repo")));
        assert_eq!(tree.files.len(), 1);
        assert_eq!(tree.dirs["src"].files.len(), 1);
        assert_eq!(tree.dirs["src"].dirs["deep"].files.len(), 1);
        assert_eq!(tree.dirs["src"].count_files(), 2);
        assert_eq!(tree.count_files(), 3);
        let mut all = Vec::new();
        tree.collect(&mut all);
        assert_eq!(all.len(), 3);
    }

    #[cfg(feature = "test-support")]
    fn recorded_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp(egui::Id::new(("fixture-target", name))))
    }

    #[cfg(feature = "test-support")]
    fn click(rect: egui::Rect, mut draw: impl FnMut(Vec<egui::Event>)) {
        let pos = rect.center();
        draw(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn git_panel_renders_icon_controls_stats_and_committed_section() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let context = ContextData {
            cwd: "/repo".into(),
            root: Some("/repo".into()),
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![
                terminator_git::Change {
                    path: "/repo/a.rs".into(),
                    status: " M".into(),
                },
                terminator_git::Change {
                    path: "/repo/new.rs".into(),
                    status: "??".into(),
                },
            ],
            decorations: Default::default(),
            stats: std::collections::HashMap::from([(PathBuf::from("/repo/a.rs"), (4, 2))]),
            error: None,
        };
        let compare = workspace_ops::CompareData {
            root: Some("/repo".into()),
            upstream: Some("origin/main".into()),
            ahead: 1,
            behind: 0,
            base: Some("origin/main".into()),
            files: vec![workspace_ops::CommittedFile {
                path: "/repo/lib.rs".into(),
                letter: 'M',
                added: 3,
                deleted: 1,
            }],
        };
        let branches = vec!["main".to_string(), "dev".to_string()];
        let theme = AppearanceConfig::default();
        let mut outcome = GitPanelOutcome::default();
        let draw = |events: Vec<egui::Event>, outcome: &mut GitPanelOutcome| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut draft = String::new();
                    let mut input = GitPanelInput {
                        context: &context,
                        review_mode: ReviewMode::Native,
                        neovim_review: false,
                        theme: &theme,
                        history: false,
                        view_list: false,
                        commits: &[],
                        branches: &branches,
                        commit_draft: &mut draft,
                        collapse_generation: 0,
                        open_shortcut: "",
                        compare: Some(&compare),
                        base_ref: None,
                    };
                    *outcome = git_panel(ui, &mut input);
                },
            );
            output.textures_delta.clear();
        };
        draw(vec![], &mut outcome);
        assert!(recorded_target(&ctx, "git-changes").is_some());
        assert!(recorded_target(&ctx, "git-history").is_some());
        assert!(recorded_target(&ctx, "git-collapse").is_some());
        assert!(recorded_target(&ctx, "git-file-a.rs").is_some());
        assert!(recorded_target(&ctx, "git-file-new.rs").is_some());
        assert!(recorded_target(&ctx, "git-committed-lib.rs").is_some());
        let stage_all = recorded_target(&ctx, "git-stage-all").expect("stage all is painted");
        click(stage_all, |events| draw(events, &mut outcome));
        assert!(outcome.stage_all);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn explorer_segmented_switches_to_contents_mode() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut mode = ExplorerSearchMode::Names;
        let draw = |events: Vec<egui::Event>, mode: &mut ExplorerSearchMode| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 80.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| explorer_segmented(ui, mode),
            );
            output.textures_delta.clear();
        };
        draw(vec![], &mut mode);
        let contents = recorded_target(&ctx, "explorer-mode-contents").expect("contents cell");
        click(contents, |events| draw(events, &mut mode));
        assert_eq!(mode, ExplorerSearchMode::Contents);
    }
}
