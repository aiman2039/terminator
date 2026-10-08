use super::super::appearance;
use eframe::egui::{self};
use egui_dock::DockArea;
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn center_pane(&mut self, ui: &mut egui::Ui) {
        self.settings_unsaved_dialog(ui.ctx());
        if self.settings_open {
            self.settings_center(ui);
            return;
        }
        if self.player_open {
            self.player_center(ui);
            return;
        }
        if self.palette_open {
            self.palette_center(ui);
            return;
        }
        if self.worktree_open {
            self.worktree_management_center(ui);
            return;
        }
        if self.worktree_draft.is_some() {
            self.worktree_center(ui);
            return;
        }
        if self.search_open {
            self.search_history_center(ui);
            return;
        }
        self.workspace_center(ui);
    }

    pub(crate) fn search_history_center(&mut self, ui: &mut egui::Ui) {
        let Some(sid) = self.search_session.clone() else {
            self.close_scrollback_search();
            return;
        };
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Search session history");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_scrollback_search();
                }
            });
        });
        ui.add_space(6.0);
        ui.add(
            appearance::singleline(&mut self.search)
                .id(egui::Id::new("scrollback-search"))
                .hint_text("Search scrollback…")
                .desired_width(400.0),
        );
        ui.add_space(8.0);
        let key = format!("history:{sid}");
        if !self.texts.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::rpc(
                Request::History {
                    session: sid.clone(),
                },
                After::Text(key.clone()),
            ));
        }
        egui::ScrollArea::both().show(ui, |ui| {
            if let Some(text) = self.texts.get(&key) {
                let needle = self.search.to_lowercase();
                for (line, text) in text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.to_lowercase().contains(&needle))
                    .take(2000)
                {
                    ui.monospace(format!("{}  {}", line.saturating_add(1), text));
                }
            } else {
                ui.weak("Loading scrollback…");
            }
        });
    }

    pub(crate) fn worktree_management_center(&mut self, ui: &mut egui::Ui) {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Worktrees");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.worktree_open = false;
                }
            });
        });
        ui.add_space(8.0);
        let has_worktrees = self.state.worktrees.iter().any(|w| !w.removed);
        if !has_worktrees && self.worktree_draft.is_none() {
            ui.weak("No worktrees yet.");
            ui.add_space(8.0);
            if ui.button("Create a worktree").clicked() {
                self.open_worktree_wizard();
            }
            return;
        }
        if let Some(draft) = &self.worktree_draft {
            let mut submit = false;
            let mut browse = false;
            let mut cancel = false;
            let mut clone = draft.clone();
            self.worktree_form(ui, &mut clone, &mut submit, &mut browse, &mut cancel);
            if browse {
                self.browse_target = Some(BrowseTarget::WorktreeDest);
            }
            if submit {
                self.submit_worktree(&clone);
            }
            if submit || cancel {
                self.worktree_draft = None;
            } else {
                self.worktree_draft = Some(clone);
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
        }
        let _header_h = 28.0;
        let footer_h = 44.0;
        let list_h = (ui.available_height() - footer_h).max(80.0);
        let worktrees: Vec<_> = self
            .state
            .worktrees
            .iter()
            .filter(|w| !w.removed)
            .cloned()
            .collect();
        let selected = self.selected.clone();
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .show(ui, |ui| {
                for worktree in &worktrees {
                    let Some(project) = self
                        .state
                        .projects
                        .iter()
                        .find(|p| p.id == worktree.project_id)
                    else {
                        continue;
                    };
                    let live = self
                        .state
                        .sessions
                        .iter()
                        .filter(|s| s.project_id == project.id && s.lifecycle.live())
                        .count();
                    let is_selected = selected.as_ref() == Some(&project.id);
                    let path = project.path.display().to_string();
                    let name = project.name.clone();
                    let project_id = project.id.clone();
                    let response = appearance::project_row(
                        ui,
                        &name,
                        "GitBranch",
                        is_selected,
                        self.theme.row_height(),
                        &live.to_string(),
                        appearance::color(&self.theme.secondary),
                    )
                    .on_hover_text(format!(
                        "{}\n{}",
                        path,
                        worktree.path.display()
                    ));
                    if response.clicked() {
                        self.select_project(project_id.clone());
                        self.worktree_open = false;
                    }
                    let app = &mut *self;
                    appearance::context_menu(&response, |ui| {
                        if appearance::menu_item(ui, "Open", "FolderOpen", "").clicked() {
                            app.select_project(project_id.clone());
                            app.worktree_open = false;
                            ui.close();
                        }
                        if appearance::menu_item(
                            ui,
                            "New terminal",
                            "Terminal",
                            &app.shortcut_label("new_terminal"),
                        )
                        .clicked()
                        {
                            app.select_project(project_id.clone());
                            app.create(None);
                            app.worktree_open = false;
                            ui.close();
                        }
                        if appearance::menu_item(ui, "Remove worktree…", "X", "").clicked() {
                            app.confirm_remove_worktree(&project_id);
                            ui.close();
                        }
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if self.worktree_draft.is_none() && ui.button("New worktree").clicked() {
                self.open_worktree_wizard();
            }
        });
    }

    pub(crate) fn workspace_center(&mut self, ui: &mut egui::Ui) {
        let Some(project) = self.selected.clone() else {
            self.workspace_empty(ui);
            return;
        };
        self.workspace_project(ui, project);
    }

    pub(crate) fn workspace_empty(&mut self, ui: &mut egui::Ui) {
        let empty = self.state.projects.is_empty();
        let setup = cfg!(target_os = "macos")
            && self
                .preferences
                .needs_setup(self.state_loaded, self.state.projects.len());
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.heading(if empty {
                    "A home for your terminals."
                } else {
                    "No project selected."
                });
                ui.label(if empty {
                    "Persistent sessions. Project layouts. Agents within reach."
                } else {
                    "Restore a project from Removed, or add a folder."
                });
                if setup {
                    return;
                }
                ui.add_space(12.0);
                if ui
                    .button(if empty {
                        "Add your first project"
                    } else {
                        "Add project"
                    })
                    .clicked()
                {
                    self.add_project = true;
                }
            });
        });
    }

    pub(crate) fn workspace_project(&mut self, ui: &mut egui::Ui, project: String) {
        let mut dock = self
            .layouts
            .remove(&project)
            .unwrap_or_else(Workspace::empty);
        self.sync_active_session(&mut dock);
        self.restore_cleared_focus(&dock);
        if dock.iter_all_tabs().next().is_none() {
            self.workspace_blank(ui);
            // No leaf to drop on. A strip shell released here still becomes
            // the main pane's first tab.
            if self.pane_drag_from_strip
                && ui.input(|input| input.pointer.any_released())
                && let Some(Tab::Terminal(sid)) = self.pane_drag.clone()
            {
                self.pull_strip_shell_into(&mut dock, &sid);
            }
        } else {
            self.paint_dock(ui, &project, &mut dock);
        }
        self.apply_focus_tab(&mut dock);
        self.apply_add_tab(&project, &mut dock);
        self.apply_detach_pane(&mut dock);
        self.apply_dock_back(&mut dock);
        self.apply_float_pane(&project, &mut dock);
        self.paint_session_focus(ui, &dock);
        self.layouts.insert(project.clone(), dock);
        if self.pending_layout_save {
            self.pending_layout_save = false;
            self.note_focus_baselines();
            self.save_layouts();
        }
        self.drain_main_to_strip();
        self.paint_drag_ghost(ui);
        self.paint_tab_ghost(ui);
        // A release the docks did not accept cancels so the payload cannot stick.
        if self.pane_drag.is_some() && ui.input(|i| i.pointer.any_released()) {
            self.drop_preview_origin = None;
            self.end_pane_drag();
        }
        // Pane "Close tab" is queued while this workspace is checked out.
        self.drain_pending_unavailable_close();
    }

    pub(crate) fn follow_focus_tab(&mut self, tab: Option<Tab>) {
        match tab {
            Some(Tab::Terminal(sid)) => {
                self.active_session = Some(sid);
                self.non_terminal_selected = false;
            }
            Some(_) => {
                self.active_session = None;
                self.non_terminal_selected = true;
            }
            None => {}
        }
    }

    pub(crate) fn sync_active_session(&mut self, dock: &mut Workspace) {
        let focused = dock
            .main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| tab.clone());
        let main_moved = focused != self.last_main_focus;
        if main_moved {
            self.last_main_focus.clone_from(&focused);
            self.follow_focus_tab(focused);
        }
        let strip_focused = self
            .selected
            .as_ref()
            .and_then(|project| self.preferences.ide_strip_docks.0.get(project))
            .and_then(|strip| {
                let surface = strip.main_surface();
                surface
                    .focused_leaf()
                    // Checked: removing the last tab empties the tree while
                    // focus goes stale, and blind indexing would panic.
                    .and_then(|node| surface.leaf(node).ok())
                    .and_then(|leaf| leaf.tabs.get(leaf.active.0))
                    .cloned()
            });
        let strip_moved = strip_focused != self.last_strip_focus;
        self.last_strip_focus.clone_from(&strip_focused);
        // Both docks move together when the project changes. Keep the main
        // pane; a later strip-only move can still take focus.
        if strip_moved && !main_moved && self.ide_strip_visible() {
            self.follow_focus_tab(strip_focused);
        }
    }

    pub(crate) fn workspace_blank(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading("Your workspace, ready.");
            ui.label("Open a terminal. Run the tools you already use.");
            ui.add_space(12.0);
            if ui.button("Open terminal").clicked() {
                self.create(None);
            }
        });
    }

    pub(crate) fn dock_style(&self, ui: &egui::Ui) -> egui_dock::Style {
        let mut style = egui_dock::Style::from_egui(ui.style());
        style.tab_bar.height = 32.0;
        style.buttons.add_tab_align = egui_dock::style::TabAddAlign::Left;
        style.separator.width = self.theme.pane_divider_width;
        style.separator.color_idle = appearance::color(&self.theme.window);
        style.main_surface_border_rounding = egui::CornerRadius::same(2);
        style.tab.tab_body.corner_radius = egui::CornerRadius::same(2);
        style
    }

    pub(crate) fn paint_dock(&mut self, ui: &mut egui::Ui, project: &str, dock: &mut Workspace) {
        let style = self.dock_style(ui);
        self.refresh_pane_maps(project, dock);
        // Issuing tab and leaf for close requests: the rendered tab
        // (mirroring `Deref` selection) and its focused leaf, captured
        // before rendering so later focus moves cannot redirect closes.
        let (tab, node) = dock
            .tabs
            .get(dock.active_index())
            .or(dock.tabs.first())
            .map(|tab| (tab.id.clone(), tab.layout.focused_leaf()))
            .unzip();
        let node: Option<egui_dock::NodePath> = node.flatten();
        DockArea::new(dock)
            .style(style)
            .show_add_buttons(true)
            .show_leaf_close_all_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show_inside(
                ui,
                &mut Viewer {
                    app: self,
                    strip: false,
                    project: Some(project.to_owned()),
                    tab,
                    node,
                },
            );
        self.finish_pane_drop(ui, dock);
    }
}
