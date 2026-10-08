use super::attention::state_color;
use super::explorer_rows::{explorer_file_row, skip_clipped_row};
use super::git_panel::{GitPanelInput, PreparedGit, git_color, git_panel};
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    App, RenameSurface, Tab, appearance,
    preferences::{ExplorerSearchMode, SidebarTool},
    workspace_ops,
};
use eframe::egui::{self, RichText};
use std::path::Path;
use terminator_core::{
    AgentState, NVIM_REVIEW_CAPABILITY, Project, Request, Session, SessionKind, now,
};

impl App {
    pub(crate) fn explorer_tooltip(&self) -> String {
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

    pub(crate) fn sidebar_project(&self, project: &Project) -> std::sync::Arc<Project> {
        let mut cached = self.sidebar_projects.borrow_mut();
        let entry = cached
            .entry(project.id.clone())
            .or_insert_with(|| std::sync::Arc::new(project.clone()));
        if entry.as_ref() != project {
            *entry = std::sync::Arc::new(project.clone());
        }
        std::sync::Arc::clone(entry)
    }

    pub(crate) fn visible_projects(&self) -> Vec<std::sync::Arc<Project>> {
        self.cached_projects().as_ref().clone()
    }

    /// Brand plus status for a project row when one of its live sessions has an agent.
    /// A running agent wins over a finished one, so a working logo is not replaced by a folder.
    pub(super) fn project_agent_face(
        &self,
        project_id: &str,
    ) -> Option<(&'static str, AgentState)> {
        self.sidebar_index().faces.get(project_id).copied()
    }

    pub(super) fn session_row(&mut self, ui: &mut egui::Ui, session: &Session) {
        let presented = self.present_session(&session.id);
        let editing = self.renaming(&session.id, RenameSurface::Sidebar);
        let height = if !editing && presented.notice_preview.is_some() {
            appearance::SESSION_ROW_DETAIL_HEIGHT
        } else {
            appearance::SESSION_ROW_HEIGHT
        };
        if !editing && skip_clipped_row(ui, height) {
            return;
        }
        let index = self.sidebar_index();
        let terminal_note = index.terminal_notes.get(&session.id);
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
                    egui::pos2(response.rect.min.x + 36.0, response.rect.min.y + 4.0),
                    egui::pos2(response.rect.max.x - 6.0, response.rect.max.y - 3.0),
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

    pub(super) fn worktree_card(&mut self, ui: &mut egui::Ui, project: &Project) {
        let index = self.sidebar_index();
        let live = index.live_counts.get(&project.id).copied().unwrap_or(0);
        let (waiting, running) = index
            .worktree_states
            .get(&project.id)
            .copied()
            .unwrap_or_default();
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

    pub(crate) fn tree(&mut self, ui: &mut egui::Ui, path: &std::path::Path, _depth: usize) {
        ui.spacing_mut().interact_size.y = 24.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        let rows = self.explorer_rows(path);
        let open_shortcut = self.shortcut_label("open_file");
        let split_shortcut = self.shortcut_label("split_right");
        for row in rows.iter() {
            let mut rect = ui.available_rect_before_wrap();
            // Explorer indent is a UI coordinate; depth can exceed the f32 mantissa.
            #[allow(clippy::cast_precision_loss)]
            let indent = row.depth as f32 * ui.spacing().indent;
            rect.min.x += indent;
            ui.push_id(&row.path, |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    if let Some(entry) = &row.entry {
                        let height = 24.0;
                        if skip_clipped_row(ui, height) {
                            return;
                        }
                        self.explorer_entry(ui, entry, &row.label, &open_shortcut, &split_shortcut);
                    } else {
                        self.explorer_directory_status(ui, &row.path);
                    }
                });
            });
        }
    }

    fn explorer_directory_status(&mut self, ui: &mut egui::Ui, path: &Path) {
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
        if !self.dirs.contains_key(path) && !self.directory_errors.contains_key(path) {
            ui.weak("Loading…");
        }
    }

    fn explorer_entry(
        &mut self,
        ui: &mut egui::Ui,
        entry: &terminator_git::Entry,
        label: &str,
        open_shortcut: &str,
        split_shortcut: &str,
    ) {
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
                label,
                if expanded { "FolderOpen" } else { "Folder" },
                false,
                22.0,
                &status.to_string(),
                color,
            );
            // Build the tooltip lazily: `format!` per visible row per
            // frame showed up as scroll-time allocations.
            let folder = if folder.hovered() {
                folder.on_hover_text(format!(
                    "{}\n{}",
                    entry.path.display(),
                    if entry.ignored {
                        "Ignored"
                    } else {
                        terminator_git::status_description(status)
                    }
                ))
            } else {
                folder
            };
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
                    self.open_name_prompt(workspace_ops::NamePrompt::File {
                        dir: folder_path.clone(),
                        name: String::new(),
                    });
                    ui.close();
                }
                if appearance::menu_item(ui, "New folder", "Folder", "").clicked() {
                    self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                        dir: folder_path.clone(),
                        name: String::new(),
                    });
                    ui.close();
                }
                if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "").clicked() {
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
        } else {
            let status = self
                .context
                .as_ref()
                .and_then(|c| c.decorations.get(&entry.path))
                .copied()
                .unwrap_or(' ');
            let outcome = explorer_file_row(
                ui,
                &entry.path,
                label,
                status,
                entry.ignored,
                &self.theme,
                open_shortcut,
                split_shortcut,
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

    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        if self.preferences.tool == SidebarTool::Info {
            self.info_panel(ui);
            return;
        }
        if self.preferences.tool == SidebarTool::History {
            let index = self.sidebar_index();
            let groups = self.cached_history();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let width = (ui.available_width() - 160.0).max(60.0);
                let response = ui.add_sized(
                    egui::vec2(width, 22.0),
                    appearance::singleline(&mut self.preferences.history_filter)
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
        if let Some(context) = self.context.take() {
            let prepared = {
                let cache = self.sidebar_cache.get_mut();
                let git = cache
                    .git
                    .get_or_insert_with(|| std::sync::Arc::new(PreparedGit::new(&context)));
                std::sync::Arc::clone(git)
            };
            self.ensure_git_lists(context.root.clone());
            let open_shortcut = self.shortcut_label("open_file");
            let mut draft = std::mem::take(&mut self.git_commit);
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
            self.context = Some(context);
            self.perform_git_outcome(ui, outcome);
        } else {
            ui.weak("Select a terminal to inspect its context.");
        }
    }
}
