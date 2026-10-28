//! The project list: toolbar, project rows, and their worktrees.
use super::*;

impl App {
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
                if appearance::sidebar_action(ui, "Monitor", "New SSH project").clicked() {
                    let generation = self.remote_ui.open_new();
                    let _ = self.jobs.send(Job::SshConfigHosts(generation));
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
                        .id(ui.scope_id().with("project-filter"))
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
                                    .tint(appearance::icon_color())
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
                            appearance::project_row(appearance::ProjectRowInput {
                                ui,
                                label: &p.name,
                                icon: if expanded { "FolderOpen" } else { "Folder" },
                                selected,
                                height: self.theme.row_height(),
                                trailing: &count.to_string(),
                                tint: appearance::color(&self.theme.secondary),
                            })
                        }
                        .on_hover_text(format!(
                            "{}\n{}{}",
                            p.name,
                            self.project_location_label(&p.id),
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
}
