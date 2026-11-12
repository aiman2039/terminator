//! The project list: toolbar, project rows, and their worktrees.
use super::*;
use crate::app::missing_folder::{MISSING_ICON, MISSING_TOOLTIP};
use terminator_core::Project;

impl App {
    pub(crate) fn projects(&mut self, ui: &mut egui::Ui) {
        let sessions = &self.state.sessions;
        self.session_selection.begin_frame();
        self.session_selection
            .retain(|sid| sessions.iter().any(|s| s.id == sid && s.lifecycle.live()));
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
        let list = appearance::sidebar_scroll("projects").show(ui, |ui| {
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
        self.project_folder_drop(ui, list.inner_rect);
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
                self.show_center(crate::CenterPage::Worktrees);
            }
        }
        if appearance::sidebar_action(ui, "Monitor", "New SSH host").clicked() {
            self.new_ssh_host();
        }
        let add = appearance::sidebar_action(ui, "Plus", "New project");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-add", add.rect);
        if add.clicked() {
            self.request_pick(crate::PickKind::Project);
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
                    // A worktree removed from the sidebar leaves it here too.
                    if !self.preferences.hidden_projects.contains(&child.id) {
                        self.worktree_card(ui, child);
                    }
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
        if self
            .project_names
            .draft
            .as_ref()
            .is_some_and(|draft| draft.id == p.id)
        {
            let left = ui.next_widget_position().x;
            self.project_rename_field(ui);
            return left;
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
        self.project_row_dnd(ui, &response, &p.id);
        response.rect.left()
    }

    /// A warning icon and a dim name when the folder is missing, else the
    /// project's folder (or chosen icon and color) and name. Live agents
    /// show as state counters in place of the terminal count; the agent
    /// brand stays on the session rows.
    fn project_row(&self, ui: &mut egui::Ui, p: &Project, state: (usize, bool)) -> egui::Response {
        let (count, expanded) = state;
        let selected = self.selected.as_ref() == Some(&p.id);
        if self.project_folder_missing(&p.id) {
            return self.missing_project_row(ui, p, (count, selected));
        }
        let counters = self.project_state_counters(&p.id);
        let extras = appearance::RowExtras {
            counters: &counters,
            ..appearance::RowExtras::default()
        };
        let folder = if expanded { "FolderOpen" } else { "Folder" };
        appearance::project_row_with(
            appearance::ProjectRowInput {
                ui,
                label: &p.name,
                icon: self.project_icon(&p.id, folder),
                color: self.project_color(&p.id),
                selected,
                height: self.theme.row_height(),
                trailing: &count.to_string(),
                tint: appearance::color(&self.theme.secondary),
            },
            extras,
        )
    }

    /// The project's live agents as row state counters.
    pub(crate) fn project_state_counters(&self, project: &str) -> Vec<appearance::StateCounter> {
        let counts = self
            .sidebar_index()
            .state_counts
            .get(project)
            .copied()
            .unwrap_or_default();
        state_counters(counts, &self.theme)
    }

    /// `state` is the live-terminal count and the selected flag.
    fn missing_project_row(
        &self,
        ui: &mut egui::Ui,
        p: &Project,
        state: (usize, bool),
    ) -> egui::Response {
        let (count, selected) = state;
        let response = ui
            .scope(|ui| {
                ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                appearance::project_row(appearance::ProjectRowInput {
                    ui,
                    label: &p.name,
                    icon: MISSING_ICON,
                    color: None,
                    selected,
                    height: self.theme.row_height(),
                    trailing: &count.to_string(),
                    tint: appearance::color(&self.theme.secondary),
                })
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("project-missing:{}", p.id),
            response.rect,
        );
        response
    }

    pub(crate) fn project_hover(&self, p: &Project) -> String {
        format!(
            "{}{}\n{}{}",
            if self.project_folder_missing(&p.id) {
                format!("{MISSING_TOOLTIP}\n")
            } else {
                String::new()
            },
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
        self.project_style_menu(ui, p);
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
        self.init_command_item(ui, p);
        self.restore_workspace_item(ui, &p.id);
        if self.has_worktrees() || self.has_project_init() {
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
