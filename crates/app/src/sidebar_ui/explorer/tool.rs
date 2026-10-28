//! Sidebar tool dispatch: History, Explorer, and Git hosts.
use super::*;

impl App {
    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let tool = self.preferences.tool;
        self.sidebar_tool(tool, ui);
    }

    /// One tool's content, hostable by either sidebar.
    pub(crate) fn sidebar_tool(&mut self, tool: SidebarTool, ui: &mut egui::Ui) {
        if self.selected_remote_project().is_some()
            && matches!(
                tool,
                SidebarTool::Explorer
                    | SidebarTool::Git
                    | SidebarTool::Structure
                    | SidebarTool::Problems
                    | SidebarTool::Info
            )
        {
            self.remote_sidebar(ui, tool);
            return;
        }
        if tool == SidebarTool::Info {
            self.info_panel(ui);
            return;
        }
        if tool == SidebarTool::History {
            let index = self.sidebar_index();
            let groups = self.cached_history();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let width = (ui.available_width() - 160.0).max(60.0);
                let response = ui.add_sized(
                    egui::vec2(width, 22.0),
                    appearance::singleline(&mut self.preferences.history_filter)
                        .id(ui.scope_id().with("history-filter"))
                        .hint_text("Filter by name"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "history-filter", response.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &response;
                if !self.preferences.history_filter.is_empty()
                    && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
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
                    for group in groups.iter() {
                        self.preferences
                            .history_expanded
                            .insert(group.project.id.clone(), !all_expanded);
                    }
                }
                self.history_sort_menu(ui);
            });
            ui.add_space(4.0);
            appearance::sidebar_scroll("global-history").show(ui, |ui| {
                if index.resumable.is_empty() {
                    ui.weak("No resumable sessions.");
                } else if groups.is_empty() {
                    ui.weak("No matching sessions.");
                }
                for group in groups.iter() {
                    let expanded = *self
                        .preferences
                        .history_expanded
                        .entry(group.project.id.clone())
                        .or_insert(true);
                    let header = appearance::row(appearance::RowInput {
                        ui,
                        label: &group.project.name,
                        icon: if expanded {
                            "ChevronDown"
                        } else {
                            "ChevronRight"
                        },
                        selected: false,
                        height: 26.0,
                        trailing: &group.sessions.len().to_string(),
                        tint: appearance::color(&self.theme.text),
                    })
                    .on_hover_text(self.project_location_label(&group.project.id));
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
        if tool == SidebarTool::Problems {
            self.problems_panel(ui);
            return;
        }
        if tool == SidebarTool::Structure {
            self.structure_panel(ui);
            return;
        }
        if tool == SidebarTool::Vnc {
            self.vnc_sidebar(ui);
            return;
        }
        if tool == SidebarTool::Hosts {
            self.hosts_sidebar(ui);
            return;
        }
        if tool == SidebarTool::Agents {
            self.agents_view(ui);
            return;
        }
        if tool == SidebarTool::Explorer {
            if self.explorer_reveal.as_ref().is_some_and(|reveal| {
                self.selected.as_deref() != Some(&reveal.project)
                    || self.focused_local_file().as_ref() != Some(&reveal.file)
            }) {
                self.explorer_reveal = None;
            }
            self.pending_delete_bar(ui);
            let host = ui.scope_id();
            self.explorer_autoreveal(host);
            // Branches for "Compare with branch" (loaded once per repo).
            let git_root = self
                .context
                .as_ref()
                .and_then(|context| context.root.clone());
            self.ensure_git_lists(git_root);
            let root_override = self
                .explorer_reveal
                .as_ref()
                .filter(|reveal| reveal.host == host)
                .map(|reveal| reveal.root.clone());
            if let Some(cwd) = root_override.or_else(|| self.cwd()) {
                self.explorer_toolbar(ui, &cwd);
                let cwd = self
                    .explorer_reveal
                    .as_ref()
                    .filter(|reveal| reveal.host == host)
                    .map_or(cwd, |reveal| reveal.root.clone());
                let contents =
                    self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
                if contents {
                    appearance::sidebar_scroll("files-search")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.explorer_results(ui, &cwd));
                } else {
                    appearance::sidebar_scroll("files")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.tree_with_host(ui, &cwd, host));
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
        self.pending_git_confirm_bar(ui);
        if let Some(context) = self.context.take() {
            if context.perforce.is_some() {
                let open_shortcut = self.shortcut_label(Action::OpenFile);
                let outcome = crate::sidebar_ui::perforce_panel::panel(
                    ui,
                    &context,
                    &self.theme,
                    &open_shortcut,
                );
                self.context = Some(context);
                self.perform_git_outcome(ui, outcome);
                return;
            }
            let prepared = {
                let cache = self.sidebar_cache.get_mut();
                let git = cache
                    .git
                    .get_or_insert_with(|| std::sync::Arc::new(PreparedGit::new(&context)));
                std::sync::Arc::clone(git)
            };
            self.ensure_git_lists(context.root.clone());
            let open_shortcut = self.shortcut_label(Action::OpenFile);
            let mut draft = std::mem::take(&mut self.git.commit);
            let recent_messages = self.preferences.commit_messages.clone();
            let outcome = git_panel(
                ui,
                &mut GitPanelInput {
                    context: &context,
                    prepared: Some(&prepared),
                    review_mode: self.state.settings.review_mode,
                    neovim_review: self
                        .state
                        .capabilities
                        .iter()
                        .any(|c| c == NVIM_REVIEW_CAPABILITY),
                    theme: &self.theme,
                    history: self.git.history,
                    view_list: self.preferences.git_view_list,
                    commits: &self.git.log,
                    shelves: &self.git.shelves,
                    branches: &self.git.branches,
                    commit_draft: &mut draft,
                    recent_messages: &recent_messages,
                    amend: &mut self.git.amend,
                    signoff: &mut self.git.signoff,
                    collapse_generation: self.git.collapse,
                    open_shortcut: &open_shortcut,
                    compare: self
                        .git
                        .compare
                        .as_ref()
                        .filter(|_| self.git.compare_root == context.root),
                    base_ref: self.git.base_ref.as_deref(),
                    committing: self.git.commit_inflight.is_some(),
                },
            );
            self.git.commit = draft;
            self.context = Some(context);
            self.perform_git_outcome(ui, outcome);
        } else {
            ui.weak("Select a terminal to inspect its context.");
        }
    }
}
