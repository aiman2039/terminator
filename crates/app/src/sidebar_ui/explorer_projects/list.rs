//! The project list: toolbar, project rows, and their worktrees.
use super::*;
use terminator_core::Project;

impl App {
    pub(crate) fn projects(&mut self, ui: &mut egui::Ui) {
        // Pin the row height so the filter field shares one baseline with
        // the icon buttons.
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.projects_toolbar(ui);
            });
        });
        ui.spacing_mut().item_spacing.y = 0.0;
        let index = self.sidebar_index();
        let projects = self.cached_projects();
        let visible_ids: std::collections::HashSet<_> =
            projects.iter().map(|p| p.id.as_str()).collect();
        appearance::sidebar_scroll("projects").show(ui, |ui| {
            for p in projects.iter() {
                // Worktrees of a listed project draw under it, not on their own.
                if index.parents.get(&p.id).is_some_and(|parents| {
                    parents
                        .iter()
                        .any(|parent| visible_ids.contains(parent.as_str()))
                }) {
                    continue;
                }
                let children = index.children.get(&p.id).map(Vec::as_slice);
                let count = index.live_counts.get(&p.id).copied().unwrap_or(0);
                self.project_entry(ui, p, (count, children));
            }
            if projects.is_empty() {
                self.projects_empty_hint(ui);
            }
        });
    }

    /// Removed, sort, worktrees, SSH, new project, and the filter field
    /// (right to left).
    fn projects_toolbar(&mut self, ui: &mut egui::Ui) {
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
    }

    fn projects_empty_hint(&self, ui: &mut egui::Ui) {
        ui.weak(if self.state.projects.is_empty() {
            "Add a folder to begin."
        } else if !self.preferences.project_filter.trim().is_empty() {
            "No matching projects."
        } else {
            "Restore a project from Removed."
        });
    }

    /// One project: its header row, then (expanded) its sessions and
    /// worktree cards. `rows` is the live-terminal count and the child
    /// worktrees.
    fn project_entry(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        rows: (usize, Option<&[Arc<Project>]>),
    ) {
        let (count, children) = rows;
        ui.add_space(6.0);
        let mut expanded = *self
            .preferences
            .expanded
            .entry(p.id.clone())
            .or_insert(true);
        let project_left = ui
            .horizontal(|ui| self.project_header(ui, p, (count, &mut expanded)))
            .inner;
        if !expanded || self.preferences.hidden_projects.contains(&p.id) {
            return;
        }
        ui.scope(|ui| {
            // Indent from the project row, past its separate expand button.
            ui.spacing_mut().indent += project_left - ui.next_widget_position().x;
            ui.indent(&p.id, |ui| {
                self.project_sessions_tree(ui, &p.id);
                for child in children.unwrap_or_default() {
                    self.worktree_card(ui, child);
                }
            });
        });
    }

    /// A frameless chevron that expands or collapses a sidebar node.
    /// Returns true when clicked.
    pub(super) fn sidebar_chevron(&self, ui: &mut egui::Ui, expanded: bool, hover: &str) -> bool {
        ui.add_sized(
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
        .on_hover_text(hover)
        .clicked()
    }

    /// Chevron, project row, and its menu. `state` is the live-terminal
    /// count and the expanded flag. Returns the row's left edge.
    fn project_header(&mut self, ui: &mut egui::Ui, p: &Project, state: (usize, &mut bool)) -> f32 {
        let (count, expanded) = state;
        if self.sidebar_chevron(ui, *expanded, "Expand or collapse project") {
            self.finish_rename(true);
            *expanded = !*expanded;
            self.preferences.expanded.insert(p.id.clone(), *expanded);
        }
        let response = self
            .project_row(ui, p, (count, *expanded))
            .on_hover_text(self.project_hover(p));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("project-row:{}", p.id), response.rect);
        if response.clicked() {
            self.select_project(p.id.clone());
        }
        appearance::context_menu(&response, |ui| self.project_menu(ui, p));
        response.rect.left()
    }

    /// The agent face when a live session runs an agent, else a folder.
    fn project_row(&self, ui: &mut egui::Ui, p: &Project, state: (usize, bool)) -> egui::Response {
        let (count, expanded) = state;
        let selected = self.selected.as_ref() == Some(&p.id);
        if let Some((brand, agent)) = self.project_agent_face(&p.id) {
            return appearance::session_row_spec(
                ui,
                appearance::SessionRowSpec {
                    label: &p.name,
                    icon: attention_status_icon(agent),
                    selected,
                    trailing: &count.to_string(),
                    tint: appearance::color(&self.theme.secondary),
                    icon_tint: Some(state_color(agent, &self.theme)),
                    spin: agent == AgentState::Running,
                    subtitle: None,
                    brand: Some(brand),
                },
            );
        }
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

    fn project_hover(&self, p: &Project) -> String {
        format!(
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
        )
    }

    fn project_menu(&mut self, ui: &mut egui::Ui, p: &Project) {
        if self.has_worktrees()
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "New task worktree…",
                icon: "GitBranch",
                shortcut: "",
            })
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
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Remove worktree…",
                icon: "X",
                shortcut: "",
            })
            .clicked()
        {
            self.confirm_remove_worktree(&p.id);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Remove project from sidebar",
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.hide_project(&p.id);
            ui.close();
        }
    }
}
