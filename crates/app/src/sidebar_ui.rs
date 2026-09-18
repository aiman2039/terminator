//! Project, file, Git, and notification sidebar rendering.
use super::*;
use std::cmp::Reverse;

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
    pub(super) fn agent_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.player_toggle_button(ui);
            let waiting = self.waiting_notice_count();
            let unread = self
                .state
                .notifications
                .iter()
                .filter(|notice| !notice.read && notice_pending(notice, now()))
                .count();
            let badge = match (waiting, unread) {
                (0, 0) => String::new(),
                (0, unread) => format!("{unread} unread"),
                (waiting, 0) => format!("{waiting} waiting"),
                (waiting, unread) => format!("{waiting} waiting · {unread} unread"),
            };
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
            .on_hover_text(
                "Pending agent notifications. Click to switch between Agents and Projects.",
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Agents")
            });
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "left-agent-bar", response.rect);
            if response.clicked() {
                self.preferences.left_agents = !self.preferences.left_agents;
            }
        });
        self.player_live_controls(ui);
        ui.separator();
    }
    fn project_sort_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let current = self.preferences.project_sort;
        let menu = appearance::menu_button(ui, "Sort", |ui| {
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
        })
        .response
        .on_hover_text("Sort projects by name or latest activity");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-sort", menu.rect);
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
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let menu = appearance::menu_button(ui, "Removed", |ui| {
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
        })
        .response
        .on_hover_text("Restore a project to the sidebar");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "removed-projects", menu.rect);
        let _ = menu;
    }

    pub(super) fn visible_projects(&self) -> Vec<Project> {
        sort_visible_projects(VisibleProjects {
            projects: self.state.projects.clone(),
            hidden: &self.preferences.hidden_projects,
            sort: self.preferences.project_sort,
            activity: &self.preferences.project_activity,
            sessions: &self.state.sessions,
            agents: &self.state.agents,
            notifications: &self.state.notifications,
            terminal_notices: &self.state.terminal_notices,
        })
    }
    pub(super) fn projects(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("PROJECTS").small().weak().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                self.removed_projects_menu(ui);
                self.project_sort_menu(ui);
                if self.has_worktrees() {
                    let worktree = appearance::sidebar_action(ui, "GitBranch", "New task worktree");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "worktree-add", worktree.rect);
                    if worktree.clicked() {
                        self.open_worktree_wizard();
                    }
                }
                let add = appearance::sidebar_action(ui, "Plus", "Add local project");
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
        appearance::sidebar_scroll("projects")
            .max_height((ui.available_height() - footer).max(0.0))
            .show(ui, |ui| {
                for p in self.visible_projects() {
                    if self.managed_worktree(&p.id).is_some()
                        && self.visible_projects().iter().any(|parent| {
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
                    let project_left = ui.horizontal(|ui| {
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
                        let response = appearance::project_row(
                            ui,
                            &p.name,
                            if expanded { "FolderOpen" } else { "Folder" },
                            selected,
                            self.theme.row_height(),
                            &count.to_string(),
                            appearance::color(&self.theme.secondary),
                        )
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
                        diagnostics::record(ui.ctx(), &format!("project-row:{}", p.id), response.rect);
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
                            if self.managed_worktree(&p.id).is_some()
                                && appearance::menu_item(ui, "Remove worktree…", "X", "")
                                    .clicked()
                            {
                                self.confirm_remove_worktree(&p.id);
                                ui.close();
                            }
                            if appearance::menu_item(ui, "Remove project from sidebar", "X", "")
                                .on_hover_text("Keep files, layouts, and running sessions. Restore it from Removed or add the folder again.")
                                .clicked() {
                                self.hide_project(&p.id);
                                ui.close();
                            }
                        });
                        response.rect.left()
                    }).inner;
                    if expanded && !self.preferences.hidden_projects.contains(&p.id) {
                        ui.scope(|ui| {
                            // Indent from the project row, past its separate expand button.
                            ui.spacing_mut().indent += project_left - ui.next_widget_position().x;
                            ui.indent(&p.id, |ui| {
                                let sessions: Vec<_> = self
                                    .state
                                    .sessions
                                    .iter()
                                    .filter(|s| s.project_id == p.id && s.kind != SessionKind::Editor)
                                    .cloned()
                                    .collect();
                                for session in sessions
                                    .iter()
                                    .filter(|s| s.lifecycle.live() && s.kind != SessionKind::Editor)
                                {
                                    self.session_row(ui, session);
                                }
                                let children: Vec<_> = self
                                    .worktree_children(&p)
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
                if self.state.projects.iter().all(|p| self.preferences.hidden_projects.contains(&p.id)) {
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
    fn session_row(&mut self, ui: &mut egui::Ui, session: &Session) {
        let agent = self
            .state
            .agents
            .iter()
            .filter(|a| a.session_id == session.id)
            .max_by_key(|a| a.updated);
        let terminal_note = self
            .state
            .terminal_notices
            .iter()
            .rev()
            .find(|n| n.session_id == session.id);
        let unread_terminal = terminal_note.is_some_and(|n| !n.dismissed);
        let color = agent
            .map(|a| state_color(a.state, &self.theme))
            .unwrap_or_else(|| {
                appearance::color(if unread_terminal {
                    &self.theme.accent
                } else {
                    &self.theme.secondary
                })
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
        let response = appearance::session_row(
            ui,
            if editing { "" } else { &session.label },
            if session.kind == SessionKind::Editor {
                "FileCode"
            } else {
                "Terminal"
            },
            self.active_session.as_ref() == Some(&session.id),
            24.0,
            &if editing {
                String::new()
            } else if secondary.is_empty() {
                if agent.is_some() || unread_terminal {
                    "●".into()
                } else {
                    String::new()
                }
            } else {
                secondary.into()
            },
            color,
        )
        .on_hover_text(format!(
            "{}\n{}{}",
            session.cwd.display(),
            secondary,
            terminal_note
                .map(|n| format!("\nTerminal: {}\n{}", n.title, n.body))
                .unwrap_or_default()
        ));
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
            if appearance::menu_item(ui, "Open session", "Terminal", "").clicked() {
                self.go_session(&session.id);
                ui.close();
            }
            if session.lifecycle.live() {
                if appearance::menu_item(ui, "Close session…", "X", "").clicked() {
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
        let agent_in_project = |agent: &Agent| {
            self.state
                .sessions
                .iter()
                .any(|session| session.id == agent.session_id && session.project_id == project.id)
        };
        let waiting = self.state.agents.iter().any(|agent| {
            matches!(
                agent.state,
                AgentState::WaitingInput | AgentState::WaitingPermission
            ) && agent_in_project(agent)
        });
        let running = self
            .state
            .agents
            .iter()
            .any(|agent| agent.state == AgentState::Running && agent_in_project(agent));
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
            if appearance::menu_item(ui, "New terminal", "Terminal", "").clicked() {
                self.select_project(project.id.clone());
                self.create(None);
                ui.close();
            }
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
                        self.git_color(status)
                    };
                    if appearance::file_row(
                        ui,
                        &label,
                        if expanded { "FolderOpen" } else { "Folder" },
                        false,
                        24.0,
                        &status.to_string(),
                        color,
                    )
                    .on_hover_text(format!(
                        "{}\n{}",
                        entry.path.display(),
                        if entry.ignored {
                            "Ignored"
                        } else {
                            services::status_description(status)
                        }
                    ))
                    .clicked()
                    {
                        if expanded {
                            self.expanded_dirs.remove(&entry.path);
                        } else {
                            self.expanded_dirs.insert(entry.path.clone());
                        }
                    }
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
                    let color = if entry.ignored {
                        appearance::color(&self.theme.git_ignored)
                    } else {
                        self.git_color(status)
                    };
                    let r = appearance::file_row(
                        ui,
                        &label,
                        icons::file_icon(&entry.path),
                        false,
                        24.0,
                        &status.to_string(),
                        color,
                    )
                    .on_hover_text(format!(
                        "{}\n{}",
                        entry.path.display(),
                        if entry.ignored {
                            "Ignored"
                        } else {
                            services::status_description(status)
                        }
                    ));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("explorer-file:{}", label), r.rect);
                    let pointer = FilePointer {
                        path: entry.path.clone(),
                        deleted: false,
                        staged: None,
                    };
                    self.file_pointer_action(&r, pointer);
                    appearance::context_menu(&r, |ui| {
                        if let Some(action) = file_actions::menu(
                            ui,
                            file_actions::FileMenu {
                                file: true,
                                browser: file_actions::browser_document(&entry.path),
                                git: None,
                                neovim: false,
                            },
                        ) {
                            self.file_action(ui, action, &entry.path, None);
                        }
                    });
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

    pub(super) fn side_attention_visible(&self) -> bool {
        self.state.settings.notifications_side && !self.right_agents_inbox()
    }

    fn right_agents_inbox(&self) -> bool {
        self.preferences.visible && self.preferences.tool == SidebarTool::Agents
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
            for notice in notices {
                let session = self
                    .state
                    .sessions
                    .iter()
                    .find(|session| session.id == notice.session_id)
                    .cloned();
                let highlight = self.detail.as_ref() == Some(&notice.id);
                let selected = self.active_session.as_ref() == Some(&notice.session_id);
                let action = attention_card(
                    ui,
                    AttentionCard {
                        theme: &self.theme,
                        notice: &notice,
                        session: session.as_ref(),
                        selected,
                        highlight,
                    },
                );
                self.apply_notice_action(notice.id, action);
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
    fn agents_empty(&mut self, ui: &mut egui::Ui) {
        if self.hook_status.is_empty() {
            ui.weak("Checking agent hooks…");
            return;
        }
        if self.state.agents.is_empty() && !self.hook_status.values().any(|installed| *installed) {
            ui.weak("Agent hooks are not configured");
            if ui.small_button("Set up hooks").clicked() {
                self.settings_draft = self.state.settings.clone();
                self.editor_preset = external_editor::selected(&self.settings_draft);
                self.theme_draft = self.theme_committed.clone();
                self.settings_section = SettingsSection::AgentHooks;
                self.settings_open = true;
                let _ = self.jobs.send(Job::HookStatus);
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
    pub(super) fn apply_notice_action(&mut self, id: String, action: AttentionAction) {
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
                self.detail = None;
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
                }
                if self.detail.as_ref() == Some(&id) {
                    self.detail = None;
                }
            }
        }
    }
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        if self.preferences.tool == SidebarTool::History {
            ui.heading("History");
            ui.weak("Ended sessions from all projects");
            let ended: Vec<_> = self
                .state
                .sessions
                .iter()
                .filter(|s| !s.lifecycle.live())
                .cloned()
                .collect();
            appearance::sidebar_scroll("global-history").show(ui, |ui| {
                if ended.is_empty() {
                    ui.weak("No ended sessions.");
                }
                for project in self.state.projects.clone() {
                    let sessions: Vec<_> = ended
                        .iter()
                        .filter(|s| s.project_id == project.id)
                        .collect();
                    if sessions.is_empty() {
                        continue;
                    }
                    let expanded = *self
                        .preferences
                        .history_expanded
                        .entry(project.id.clone())
                        .or_insert(true);
                    let header = appearance::row(
                        ui,
                        &project.name,
                        if expanded {
                            "ChevronDown"
                        } else {
                            "ChevronRight"
                        },
                        false,
                        26.0,
                        &sessions.len().to_string(),
                        appearance::color(&self.theme.text),
                    )
                    .on_hover_text(project.path.display().to_string());
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("history-project:{}", project.id),
                        header.rect,
                    );
                    if header.clicked() {
                        self.preferences
                            .history_expanded
                            .insert(project.id.clone(), !expanded);
                    }
                    if expanded {
                        ui.indent(("history-project", &project.id), |ui| {
                            for session in sessions {
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
            ui.checkbox(&mut self.preferences.show_ignored, "Show ignored files")
                .on_hover_text("Show files excluded by Git ignore rules and Git metadata");
            if let Some(cwd) = self.cwd() {
                appearance::sidebar_scroll("files")
                    .max_height(ui.available_height())
                    .show(ui, |ui| self.tree(ui, &cwd, 0));
            }
            return;
        }
        let heading = ui.heading("Git");
        let _ = match self.cwd() {
            Some(cwd) => heading.on_hover_text(cwd.display().to_string()),
            None => heading,
        };
        if self.context_session().is_some_and(|s| !s.cwd_confirmed) {
            ui.label(RichText::new("Last known directory").small().weak());
        }
        ui.separator();
        if self.watch_fallback {
            ui.weak("Filesystem watch unavailable; refreshing every 3 seconds");
        }
        ui.label(RichText::new("GIT STATUS").text_style(egui::TextStyle::Name("Section".into())));
        if let Some(context) = self.context.clone() {
            if context.root.is_none() {
                ui.weak("Not a Git repository");
            } else {
                ui.label(
                    RichText::new(&context.branch)
                        .color(appearance::color(&self.theme.status_running)),
                );
                if context.changes.is_empty() {
                    ui.weak("Working tree clean");
                }
                appearance::sidebar_scroll("git").show(ui, |ui| {
                    for group in services::GitGroup::ALL {
                        let entries: Vec<_> = context
                            .changes
                            .iter()
                            .filter(|c| c.in_group(group))
                            .collect();
                        if entries.is_empty() {
                            continue;
                        }
                        egui::CollapsingHeader::new(format!(
                            "{}  {}",
                            group.label(),
                            entries.len()
                        ))
                        .id_salt((context.root.clone(), group.label()))
                        .default_open(true)
                        .show(ui, |ui| {
                            for change in entries {
                                // Keep layout height without constructing thousands of off-screen
                                // buttons, labels, tooltips, and context menus on every frame.
                                if skip_clipped_git_row(ui) {
                                    continue;
                                }
                                let name = change
                                    .path
                                    .strip_prefix(context.root.as_ref().unwrap())
                                    .unwrap_or(&change.path)
                                    .display()
                                    .to_string();
                                let letter = change.letter(group);
                                let response = appearance::file_row(
                                    ui,
                                    &name,
                                    icons::file_icon(&change.path),
                                    false,
                                    24.0,
                                    &letter.to_string(),
                                    self.git_color(letter),
                                )
                                .on_hover_text(format!(
                                    "{}\n{} ({})",
                                    change.path.display(),
                                    services::status_description(letter),
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
                                self.file_pointer_action(
                                    &response,
                                    FilePointer {
                                        path: change.path.clone(),
                                        deleted: letter == 'D',
                                        staged: (!change.conflict())
                                            .then_some(group == services::GitGroup::Staged),
                                    },
                                );
                                appearance::context_menu(&response, |ui| {
                                    if let Some(action) = file_actions::menu(
                                        ui,
                                        file_actions::FileMenu {
                                            file: true,
                                            browser: file_actions::browser_document(&change.path),
                                            git: Some(group),
                                            neovim: true,
                                        },
                                    ) {
                                        self.file_action(ui, action, &change.path, None);
                                    }
                                });
                            }
                        });
                    }
                });
            }
            if let Some(e) = context.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), e);
            }
        } else {
            ui.weak("Select a terminal to inspect its context.");
        }
    }
    fn git_color(&self, status: char) -> Color32 {
        appearance::color(match status {
            'A' => &self.theme.git_added,
            'M' => &self.theme.git_modified,
            'D' | '!' => &self.theme.git_deleted,
            'R' | 'C' | 'U' => &self.theme.git_untracked,
            _ => &self.theme.secondary,
        })
    }
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
}

pub(super) struct AttentionCard<'a> {
    pub theme: &'a AppearanceConfig,
    pub notice: &'a Notification,
    pub session: Option<&'a Session>,
    pub selected: bool,
    pub highlight: bool,
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

fn notice_pending(notice: &Notification, timestamp: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= timestamp
}

fn notice_preview(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser, TagEnd};
    let mut text = String::new();
    for event in Parser::new(markdown) {
        match event {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const ATTENTION_ACTION_SIZE: f32 = 22.0;
const ATTENTION_ACTION_COUNT: f32 = 3.0;

fn attention_status(state: AgentState) -> &'static str {
    match state {
        AgentState::Completed => "Agent done",
        AgentState::WaitingInput | AgentState::WaitingPermission => "Agent waiting",
        _ => state.label(),
    }
}

fn attention_title_job(
    notice: &Notification,
    session: Option<&Session>,
    theme: &AppearanceConfig,
    muted: Color32,
    text: Color32,
) -> egui::text::LayoutJob {
    let font = egui::FontId::proportional(12.0);
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
            ..Default::default()
        },
        ..Default::default()
    };
    let format = |color: Color32| egui::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    job.append(
        attention_status(notice.state),
        0.0,
        format(state_color(notice.state, theme)),
    );
    if let Some(session) = session {
        job.append(" · ", 0.0, format(muted));
        job.append(&session.label, 0.0, format(text));
    }
    job
}

fn attention_title(
    ui: &mut egui::Ui,
    notice: &Notification,
    session: Option<&Session>,
    theme: &AppearanceConfig,
) -> egui::Response {
    let muted = ui.visuals().weak_text_color();
    let text = ui.visuals().text_color();
    let mut job = attention_title_job(notice, session, theme, muted, text);
    let natural = ui.painter().layout_job(job.clone()).size().x;
    let width = natural.min(ui.available_size_before_wrap().x.max(0.0));
    job.wrap.max_width = width;
    let galley = ui.painter().layout_job(job);
    let size = egui::vec2(width, galley.size().y.max(ATTENTION_ACTION_SIZE));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().with_clip_rect(rect).galley(
        egui::pos2(rect.min.x, rect.center().y - galley.size().y * 0.5),
        galley,
        text,
    );
    response
}

fn attention_actions(ui: &mut egui::Ui, session_id: &str) -> AttentionAction {
    let width = ATTENTION_ACTION_SIZE * ATTENTION_ACTION_COUNT;
    if ui.available_size_before_wrap().x < width {
        ui.end_row();
    }
    ui.push_id(session_id, |ui| {
        ui.horizontal(|ui| {
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size =
                egui::vec2(ATTENTION_ACTION_SIZE, ATTENTION_ACTION_SIZE);
            let mut action = AttentionAction::None;
            for (icon, tip, name, next) in [
                ("ArrowRight", "Go to context", "go", AttentionAction::Go),
                (
                    "Moon",
                    "Snooze 10 minutes",
                    "snooze",
                    AttentionAction::Snooze,
                ),
                ("X", "Dismiss", "dismiss", AttentionAction::Dismiss),
            ] {
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
    } = input;
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
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
            let action = ui
                .horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
                    let header = attention_title(ui, notice, session, theme).on_hover_ui(|ui| {
                        ui.set_max_width(360.0);
                        if let Some(session) = session {
                            ui.weak(session.cwd.display().to_string());
                        }
                        ui.label(notice_preview(&notice.summary));
                    });
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
                    let buttons = attention_actions(ui, &notice.session_id);
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
}
