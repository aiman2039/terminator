//! Project, file, Git, and notification sidebar rendering.
use super::*;

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
        let notices = self
            .state
            .notifications
            .iter()
            .filter(|n| !n.dismissed && n.snoozed_until <= now())
            .rev()
            .take(50)
            .cloned()
            .collect::<Vec<_>>();
        let terminal_notices = self
            .state
            .terminal_notices
            .iter()
            .filter(|n| !n.dismissed)
            .rev()
            .take(20)
            .cloned()
            .collect::<Vec<_>>();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "Attention  {}",
                    notices.len() + terminal_notices.len()
                ))
                .small()
                .strong()
                .color(appearance::color(&self.theme.secondary)),
            );
            if notices.is_empty() && terminal_notices.is_empty() {
                if self.hook_status.is_empty() {
                    ui.weak("Checking agent hooks…");
                } else if self.state.agents.is_empty()
                    && !self.hook_status.values().any(|installed| *installed)
                {
                    ui.weak("Agent hooks are not configured");
                    if ui.small_button("Set up hooks").clicked() {
                        self.settings_draft = self.state.settings.clone();
                        self.editor_preset = external_editor::selected(&self.settings_draft);
                        self.theme_draft = self.theme_committed.clone();
                        self.settings_section = 5;
                        self.settings_open = true;
                        let _ = self.jobs.send(Job::HookStatus);
                    }
                } else {
                    ui.weak("No pending agent events");
                }
            }
            for n in notices {
                let color = if n.resolved {
                    appearance::color(&self.theme.secondary)
                } else {
                    state_color(n.state, &self.theme)
                };
                let label = format!("● {}", n.summary.chars().take(42).collect::<String>());
                if ui
                    .button(RichText::new(label).color(color))
                    .on_hover_text(&n.summary)
                    .clicked()
                {
                    self.detail = Some(n.id.clone());
                    self.send(Request::Notice {
                        id: n.id,
                        action: "read".into(),
                    });
                }
            }
            for n in terminal_notices {
                let label = if n.title.is_empty() || n.title == "Terminal" {
                    &n.body
                } else {
                    &n.title
                };
                if ui
                    .button(format!(
                        "Terminal · {}",
                        label.chars().take(36).collect::<String>()
                    ))
                    .on_hover_text(format!("{}\n{}", n.title, n.body))
                    .clicked()
                {
                    self.go_session(&n.session_id);
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Dismiss terminal notification")
                    .clicked()
                    && self
                        .state
                        .capabilities
                        .iter()
                        .any(|c| c == TERMINAL_NOTICES_CAPABILITY)
                {
                    self.send(Request::DismissTerminalNotice { id: n.id });
                }
            }
        });
    }
    pub(super) fn agent_bar(&mut self, ui: &mut egui::Ui) {
        let waiting = self
            .state
            .agents
            .iter()
            .filter(|agent| {
                matches!(
                    agent.state,
                    AgentState::WaitingInput | AgentState::WaitingPermission
                )
            })
            .count();
        let unread = self
            .state
            .notifications
            .iter()
            .filter(|notice| !notice.read && !notice.dismissed && notice.snoozed_until <= now())
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
            "Agents",
            "Bell",
            self.preferences.left_agents,
            34.0,
            &badge,
            appearance::color(if waiting > 0 {
                &self.theme.status_waiting
            } else {
                &self.theme.text
            }),
        )
        .on_hover_text(
            "Agent notifications across all projects. Click to switch between Agents and Projects.",
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "left-agent-bar", response.rect);
        if response.clicked() {
            self.preferences.left_agents = !self.preferences.left_agents;
        }
        ui.separator();
    }
    pub(super) fn projects(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("PROJECTS").small().weak().strong());
            let add = ui.small_button("+").on_hover_text("Add local project");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "project-add", add.rect);
            if add.clicked() {
                self.add_project = true;
            }
            let hidden: Vec<_> = self
                .state
                .projects
                .iter()
                .filter(|p| self.preferences.hidden_projects.contains(&p.id))
                .cloned()
                .collect();
            if !hidden.is_empty() {
                let menu = ui
                    .menu_button("Removed", |ui| {
                        for project in hidden {
                            let response =
                                appearance::menu_item(ui, &project.name, "FolderOpen", "")
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
        });
        ui.spacing_mut().item_spacing.y = 0.0;
        let live = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live() && s.kind != SessionKind::Editor)
            .count();
        let footer = 28.0;
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - footer).max(0.0))
            .show(ui, |ui| {
                for p in self.state.projects.clone() {
                    if self.preferences.hidden_projects.contains(&p.id) { continue; }
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
                                [16.0, 28.0],
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
                            28.0,
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
                        response.context_menu(|ui| {
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
        response.context_menu(|ui| {
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
    pub(super) fn tree(&mut self, ui: &mut egui::Ui, path: &std::path::Path, depth: usize) {
        ui.spacing_mut().interact_size.y = 24.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        if depth > 20 {
            return;
        }
        self.visible_dirs.push(path.into());
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
                    if r.clicked() && !r.double_clicked() {
                        self.open_file(entry.path.clone(), None, None, false);
                    }
                    r.context_menu(|ui| {
                        if let Some(action) = file_actions::menu(ui, true, false, false) {
                            self.file_action(ui, action, &entry.path, None);
                        }
                    });
                }
            }
        } else {
            ui.weak("Loading…");
        }
    }
    pub(super) fn agents_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agents");
        ui.checkbox(&mut self.preferences.all_projects, "All projects");
        let agents = self.state.agents.clone();
        egui::ScrollArea::vertical()
            .id_salt("agents")
            .show(ui, |ui| {
                let mut count = 0;
                for agent in agents {
                    let Some(session) = self
                        .state
                        .sessions
                        .iter()
                        .find(|s| s.id == agent.session_id)
                    else {
                        continue;
                    };
                    if !self
                        .preferences
                        .includes_project(&session.project_id, self.selected.as_deref())
                    {
                        continue;
                    }
                    count += 1;
                    let label = format!(
                        "{} · {}\n{} · {}s ago",
                        agent.kind,
                        session.label,
                        agent.state.label(),
                        now().saturating_sub(agent.updated)
                    );
                    let response = ui.selectable_label(
                        self.active_session.as_ref() == Some(&agent.session_id),
                        label,
                    );
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("agent-row:{}", agent.session_id),
                        response.rect,
                    );
                    if response.clicked() {
                        self.go_session(&agent.session_id);
                    }
                }
                if count == 0 {
                    ui.weak("No observed agents in this scope.");
                }
            });
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
            egui::ScrollArea::vertical()
                .id_salt("global-history")
                .show(ui, |ui| {
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
                egui::ScrollArea::vertical()
                    .id_salt("files")
                    .max_height(ui.available_height())
                    .show(ui, |ui| self.tree(ui, &cwd, 0));
            }
            return;
        }
        ui.heading("Git");
        if let Some(cwd) = self.cwd() {
            ui.label(
                RichText::new(cwd.display().to_string())
                    .small()
                    .color(appearance::color(&self.theme.secondary)),
            );
            if self.context_session().is_some_and(|s| !s.cwd_confirmed) {
                ui.label(RichText::new("Last known directory").small().weak());
            }
            ui.separator();
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
                egui::ScrollArea::vertical().id_salt("git").show(ui, |ui| {
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
                                if response.clicked() && !response.double_clicked() {
                                    if letter == 'D' {
                                        self.add_diff(
                                            context.root.as_ref().unwrap().clone(),
                                            change.path.clone(),
                                            group == services::GitGroup::Staged,
                                        );
                                    } else {
                                        self.open_file(change.path.clone(), None, None, false);
                                    }
                                }
                                response.context_menu(|ui| {
                                    if let Some(action) = file_actions::menu(ui, true, false, true)
                                    {
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
