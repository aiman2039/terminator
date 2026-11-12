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
        match tool {
            SidebarTool::Info => self.info_panel(ui),
            SidebarTool::History => self.history_tool(ui),
            SidebarTool::Problems => self.problems_panel(ui),
            SidebarTool::Structure => self.structure_panel(ui),
            SidebarTool::Vnc => self.vnc_sidebar(ui),
            SidebarTool::Hosts => self.hosts_sidebar(ui),
            SidebarTool::Agents => self.agents_view(ui),
            SidebarTool::Explorer => self.explorer_tool(ui),
            SidebarTool::Git => self.git_tool(ui),
        }
    }

    /// Resumable sessions grouped by project, with a filter bar.
    fn history_tool(&mut self, ui: &mut egui::Ui) {
        let index = self.sidebar_index();
        let groups = self.cached_history();
        ui.horizontal(|ui| self.history_filter_bar(ui, &groups));
        ui.add_space(4.0);
        appearance::sidebar_scroll("global-history").show(ui, |ui| {
            if index.resumable.is_empty() {
                ui.weak("No resumable sessions.");
            } else if groups.is_empty() {
                ui.weak("No matching sessions.");
            }
            for group in groups.iter() {
                self.history_group(ui, group);
                ui.add_space(8.0);
            }
        });
    }

    /// Filter field, clear, expand/collapse all, and sort.
    fn history_filter_bar(
        &mut self,
        ui: &mut egui::Ui,
        groups: &[crate::preferences::HistoryGroup],
    ) {
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
        self.history_toggle_all(ui, groups);
        self.history_sort_menu(ui);
    }

    fn history_toggle_all(
        &mut self,
        ui: &mut egui::Ui,
        groups: &[crate::preferences::HistoryGroup],
    ) {
        let all_expanded = !groups.is_empty()
            && groups.iter().all(|group| {
                self.preferences
                    .history_expanded
                    .get(&group.project.id)
                    .copied()
                    .unwrap_or(true)
            });
        let (icon, tip) = if all_expanded {
            ("ChevronsDownUp", "Collapse all History projects")
        } else {
            ("ChevronDown", "Expand all History projects")
        };
        let toggle = ui
            .add_enabled_ui(!groups.is_empty(), |ui| {
                appearance::sidebar_action(ui, icon, tip)
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "history-toggle-all", toggle.rect);
        if toggle.clicked() {
            for group in groups {
                self.preferences
                    .history_expanded
                    .insert(group.project.id.clone(), !all_expanded);
            }
        }
    }

    /// One project's header and, when expanded, its session rows.
    fn history_group(&mut self, ui: &mut egui::Ui, group: &crate::preferences::HistoryGroup) {
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
    }

    /// The Explorer: toolbar, then the file tree or content search results.
    fn explorer_tool(&mut self, ui: &mut egui::Ui) {
        if self.explorer_reveal.as_ref().is_some_and(|reveal| {
            self.selected.as_deref() != Some(&reveal.project)
                || self.focused_local_file().as_ref() != Some(&reveal.file)
        }) {
            self.explorer_reveal = None;
        }
        self.pending_delete_bar(ui);
        let host = ui.scope_id();
        // A status-bar breadcrumb reveal goes to the first Explorer painted.
        if let Some(reveal) = self.explorer_reveal.as_mut()
            && reveal.host == egui::Id::NULL
        {
            reveal.host = host;
        }
        self.explorer_autoreveal(host);
        let root_override = self.explorer_reveal_root(host);
        let Some(cwd) = root_override.or_else(|| self.cwd()) else {
            return;
        };
        if let Some(gone) = self.missing_terminal_folder() {
            ui.weak(crate::app::missing_folder::terminal_folder_note(gone));
        }
        self.explorer_key_focus(ui, host);
        self.explorer_toolbar(ui, &cwd);
        let cwd = self.explorer_reveal_root(host).unwrap_or(cwd);
        if self.preferences.explorer_search_mode == ExplorerSearchMode::Contents {
            appearance::sidebar_scroll("files-search")
                .max_height(ui.available_height())
                .show(ui, |ui| self.explorer_results(ui, &cwd));
        } else {
            appearance::sidebar_scroll("files")
                .max_height(ui.available_height())
                .show(ui, |ui| self.tree_with_host(ui, &cwd, host));
        }
    }

    /// A tool window key aimed here (Cmd+1, Alt+F1): select the current
    /// file for Select In, then give the tree (or, in a contents search,
    /// the search field) keyboard focus, so arrow keys work at once.
    fn explorer_key_focus(&mut self, ui: &egui::Ui, host: egui::Id) {
        let Some(claim) = crate::shortcuts::take_tool_focus(ui.ctx()) else {
            return;
        };
        if claim.reveal && self.reveal_current_file_in_explorer(host) {
            let file = self
                .explorer_reveal
                .as_ref()
                .map(|reveal| reveal.file.clone());
            self.explorer_selection = file.iter().cloned().collect();
            self.explorer_anchor = file;
        }
        if self.preferences.explorer_search_mode == ExplorerSearchMode::Contents {
            self.explorer_focus_find = true;
        } else {
            ui.memory_mut(|memory| memory.request_focus(tree_focus_id(host)));
        }
        // The tree locks its keys from its next paint on.
        ui.ctx().request_repaint();
    }

    /// Root of a reveal aimed at this Explorer host, if any.
    fn explorer_reveal_root(&self, host: egui::Id) -> Option<PathBuf> {
        self.explorer_reveal
            .as_ref()
            .filter(|reveal| reveal.host == host)
            .map(|reveal| reveal.root.clone())
    }

    /// The Git (or Perforce) panel for the selected terminal's context.
    fn git_tool(&mut self, ui: &mut egui::Ui) {
        if self.folder_session().is_some_and(|s| !s.cwd_confirmed) {
            ui.label(RichText::new("Last known directory").small().weak());
        }
        ui.separator();
        if self.watch_fallback {
            ui.weak("Filesystem watch unavailable; refreshing every 3 seconds");
        }
        self.pending_delete_bar(ui);
        self.pending_git_confirm_bar(ui);
        let Some(context) = self.context.take() else {
            ui.weak("Select a terminal to inspect its context.");
            return;
        };
        let outcome = if context.perforce.is_some() {
            crate::sidebar_ui::perforce_panel::panel(
                ui,
                crate::sidebar_ui::perforce_panel::PerforcePanelInput {
                    context: &context,
                    theme: &self.theme,
                    // No key opens a listed file: Open file is the file picker.
                    open_shortcut: "",
                },
            )
        } else {
            self.git_panel_for(ui, &context)
        };
        self.context = Some(context);
        self.perform_git_outcome(ui, outcome);
    }

    /// Paint the Git panel over a snapshot of the app's Git state.
    fn git_panel_for(
        &mut self,
        ui: &mut egui::Ui,
        context: &crate::services::ContextData,
    ) -> super::super::git_panel::GitPanelOutcome {
        let prepared = {
            let cache = self.sidebar_cache.get_mut();
            let git = cache
                .git
                .get_or_insert_with(|| std::sync::Arc::new(PreparedGit::new(context)));
            std::sync::Arc::clone(git)
        };
        self.ensure_git_lists(context.root.clone());
        let mut draft = std::mem::take(&mut self.git.commit);
        let recent_messages = self.preferences.commit_messages.clone();
        let commit_lock = self.git_panel_lock(context);
        let outcome = git_panel(
            ui,
            &mut GitPanelInput {
                context,
                prepared: Some(&prepared),
                review_mode: self.state.settings.review_mode,
                neovim_review: self
                    .state
                    .capabilities
                    .iter()
                    .any(|c| c == NVIM_REVIEW_CAPABILITY),
                theme: &self.theme,
                tab: self.git.tab,
                view_list: self.preferences.git_view_list,
                commits: &self.git.log,
                shelves: &self.git.shelves,
                branches: &self.git.branches,
                commit_draft: &mut draft,
                recent_messages: &recent_messages,
                amend: &mut self.git.amend,
                signoff: &mut self.git.signoff,
                collapse_generation: self.git.collapse,
                tree_open: !self.git.tree_collapsed,
                // No key opens a listed file: Open file is the file picker.
                open_shortcut: "",
                compare: self
                    .git
                    .compare
                    .as_ref()
                    .filter(|_| self.git.compare_root == context.root),
                base_ref: self.git.base_ref.as_deref(),
                focus_message: self.git.focus_message,
                commit_run: self
                    .git
                    .run
                    .as_ref()
                    .map(crate::app::git_panel_state::CommitRun::progress),
                commit_lock,
                commit_failure: self.git.commit_failure.as_deref(),
                list_error: self.git.list_error.as_deref(),
            },
        );
        self.git.commit = draft;
        // A focus request is for this paint only.
        self.git.focus_message = false;
        outcome
    }
}
