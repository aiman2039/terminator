use super::super::appearance;
use crate::worktree_ui::WorktreeFormInput;
use eframe::egui::{self};
use egui_dock::DockArea;
use terminator_core::*;

use super::super::*;

pub(crate) struct PaintFloatDockInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) viewport: egui::ViewportId,
    pub(crate) project: &'a str,
    pub(crate) dock: &'a mut egui_dock::DockState<Tab>,
}

impl App {
    pub(crate) fn center_pane(&mut self, ui: &mut egui::Ui) {
        // A center page (Settings, SSH, VNC, …) painted instead of the
        // workspace ends the zoom.
        if !self.center_page_or_workspace(ui) {
            self.pane_ops.zoomed = None;
            self.pane_ops.zoom_request = None;
        }
    }

    /// Paints a center page, else the workspace. True for the workspace.
    fn center_page_or_workspace(&mut self, ui: &mut egui::Ui) -> bool {
        self.settings_unsaved_dialog(ui.ctx());
        // Search Everywhere floats above this pane (`show_palette`).
        match self.center {
            CenterPage::Workspace => {
                self.workspace_center(ui);
                return true;
            }
            CenterPage::Settings => self.settings_center(ui),
            CenterPage::Player => self.player_center(ui),
            CenterPage::Worktrees => self.worktree_management_center(ui),
            CenterPage::WorktreeDraft => self.worktree_center(ui),
            CenterPage::Search(_) => self.search_history_center(ui),
            CenterPage::Commands(_) => self.command_blocks_center(ui),
            CenterPage::LocalHistory => self.local_history_center(ui),
            CenterPage::Ssh => self.ssh_page_center(ui),
            CenterPage::Vnc => self.vnc_page_center(ui),
            CenterPage::TaskChanges => self.task_changes_center(ui),
        }
        false
    }

    pub(crate) fn worktree_management_center(&mut self, ui: &mut egui::Ui) {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Worktrees");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.hide_center_overlay();
                }
            });
        });
        ui.add_space(8.0);
        let has_worktrees = self.state.worktrees.iter().any(|w| !w.removed);
        if !has_worktrees && self.worktree_draft.is_none() {
            ui.weak("No worktrees yet.");
            ui.add_space(8.0);
            if ui.button("New task…").clicked() {
                self.open_worktree_wizard();
            }
            return;
        }
        if self.worktree_draft.is_some() {
            self.worktree_draft_form(ui);
        }
        let footer_h = 44.0;
        let list_h = (ui.available_height() - footer_h).max(80.0);
        self.worktree_list(ui, list_h);
        ui.separator();
        ui.horizontal(|ui| {
            if self.worktree_draft.is_none() && ui.button("New task…").clicked() {
                self.open_worktree_wizard();
            }
        });
    }

    /// The new-worktree form over the list, while a draft is open.
    fn worktree_draft_form(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = &self.worktree_draft else {
            return;
        };
        let mut submit = false;
        let mut browse = false;
        let mut cancel = false;
        let mut clone = draft.clone();
        self.worktree_form(WorktreeFormInput {
            ui,
            draft: &mut clone,
            submit: &mut submit,
            browse: &mut browse,
            cancel: &mut cancel,
        });
        if browse {
            self.request_pick(PickKind::Browse(BrowseTarget::WorktreeDest));
        }
        if submit {
            self.submit_worktree(&mut clone);
        }
        // Keep the form after Create until the worker reports success, so a
        // failed creation can be corrected and retried.
        self.worktree_draft = (!cancel).then_some(clone);
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);
    }

    fn worktree_list(&mut self, ui: &mut egui::Ui, list_h: f32) {
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
                    self.worktree_row(ui, worktree, selected.as_ref());
                }
            });
    }

    fn worktree_row(
        &mut self,
        ui: &mut egui::Ui,
        worktree: &worktrees::Registration,
        selected: Option<&String>,
    ) {
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == worktree.project_id)
        else {
            return;
        };
        let live = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == project.id && s.lifecycle.live())
            .count();
        let is_selected = selected == Some(&project.id);
        let path = project.path.display().to_string();
        let name = project.name.clone();
        let project_id = project.id.clone();
        let response = appearance::project_row(appearance::ProjectRowInput {
            color: None,
            ui,
            label: &name,
            icon: "GitBranch",
            selected: is_selected,
            height: self.theme.row_height(),
            trailing: &live.to_string(),
            tint: appearance::color(&self.theme.secondary),
        })
        .on_hover_text(format!("{}\n{}", path, worktree.path.display()));
        if response.clicked() {
            self.select_project(project_id.clone());
            self.close_center_page(&CenterPage::Worktrees);
        }
        appearance::context_menu(&response, |ui| self.worktree_row_menu(ui, &project_id));
    }

    fn worktree_row_menu(&mut self, ui: &mut egui::Ui, project_id: &str) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Open",
            icon: "FolderOpen",
            shortcut: "",
        })
        .clicked()
        {
            self.select_project(project_id.to_owned());
            self.close_center_page(&CenterPage::Worktrees);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New terminal",
            icon: "Terminal",
            shortcut: &self.shortcut_label(Action::NewTerminal),
        })
        .clicked()
        {
            self.select_project(project_id.to_owned());
            self.create(None);
            self.close_center_page(&CenterPage::Worktrees);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Remove worktree…",
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.confirm_remove_worktree(project_id);
            ui.close();
        }
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
                    self.request_pick(PickKind::Project);
                }
            });
        });
    }

    pub(crate) fn workspace_project(&mut self, ui: &mut egui::Ui, project: String) {
        let mut dock = self
            .layouts
            .remove(&project)
            .unwrap_or_else(Workspace::empty);
        self.pending_git_tabs.get_or_insert_default();
        self.sync_active_session(&mut dock);
        self.restore_cleared_focus(&dock);
        let outer = ui.available_rect_before_wrap();
        self.settle_zoom(&project, &mut dock);
        if dock.iter_all_tabs().next().is_none() {
            self.workspace_blank(ui);
            // No leaf to drop on. A strip shell released here still becomes
            // the main pane's first tab.
            if self.drag.pane_from_strip
                && ui.input(|input| input.pointer.any_released())
                && let Some(Tab::Terminal(sid)) = self.drag.pane.clone()
            {
                self.pull_strip_shell_into(&mut dock, &sid);
            }
        } else {
            self.paint_dock(ui, &project, &mut dock);
        }
        self.apply_main_dock_ops(&project, &mut dock);
        self.paint_session_focus(ui, &dock);
        self.check_zoom(&project, &dock);
        self.paint_zoom(ui.ctx(), outer, &project);
        self.layouts.insert(project.clone(), dock);
        self.drain_pending_git_tabs();
        self.follow_preview_replacement();
        self.apply_diff_replacement();
        if self.pending_layout_save {
            self.pending_layout_save = false;
            self.note_focus_baselines();
            self.save_layouts();
        }
        self.drain_main_to_strip();
        self.paint_drag_ghost(ui);
        self.paint_tab_ghost(ui);
        // A release the docks did not accept cancels so the payload cannot stick.
        if self.drag.pane.is_some() && ui.input(|i| i.pointer.any_released()) {
            self.end_pane_drag(None, false);
        }
        // Pane "Close tab" is queued while this workspace is checked out.
        self.drain_pending_unavailable_close();
    }

    pub(crate) fn follow_focus_tab(&mut self, tab: Option<Tab>) {
        // No focused tab is no observation: focus stays where it was.
        if tab.is_some() {
            self.focus_pane(tab.as_ref());
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

    /// Pane lookup for the floating window under render, rebuilt every
    /// time (small docks; no cache). Only the window under render may
    /// read these; the main-dock maps are untouched.
    pub(crate) fn refresh_float_pane_maps(&mut self, dock: &egui_dock::DockState<Tab>) {
        self.float_pane_by_tab = dock
            .iter_all_tabs()
            .map(|(path, tab)| (tab.key(), path.node_path()))
            .collect();
        self.float_pane_tabs = dock
            .iter_leaves()
            .map(|(path, leaf)| (path, leaf.tabs.clone()))
            .collect();
    }

    /// Renders one floating window's dock with the normal theme, tab bars,
    /// and pane controls. Cross-window drag-drop is unsupported, so no
    /// drop landing runs here; a main-dock drag released over a float is
    /// cancelled by the main window's own release handling.
    /// Returns the window's dock-back requests for the caller to apply.
    pub(crate) fn paint_float_dock(
        &mut self,
        input: PaintFloatDockInput<'_>,
    ) -> Vec<super::docking::DockOp> {
        let PaintFloatDockInput {
            ui,
            viewport,
            project,
            dock,
        } = input;
        let style = appearance::dock_style(ui.style(), &self.theme);
        self.refresh_float_pane_maps(dock);
        // Diff and merge keys follow this window's focused pane.
        self.float_focus = dock
            .main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| tab.clone());
        DockArea::new(dock)
            .id(egui::Id::unique(("float-dock", viewport)))
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
                    tab: None,
                    window: Some(viewport),
                    render_path: None,
                    render_tab: None,
                },
            );
        self.float_focus = None;
        self.apply_float_dock_ops(viewport, project, dock)
    }

    pub(crate) fn paint_dock(&mut self, ui: &mut egui::Ui, project: &str, dock: &mut Workspace) {
        self.drag.drop_tabs.clear();
        let style = appearance::dock_style(ui.style(), &self.theme);
        self.refresh_pane_maps(project, dock);
        // Issuing tab for close requests: the rendered tab (mirroring
        // `Deref` selection). The issuing pane needs no capture here:
        // the dock reports each rendered leaf to the viewer directly,
        // so a focus move mid-frame cannot misattribute a close.
        let tab = dock
            .tabs
            .get(dock.active_index())
            .or(dock.tabs.first())
            .map(|tab| tab.id.clone());
        DockArea::new(dock)
            .id(egui::Id::unique(("main-dock", project, &tab)))
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
                    window: None,
                    render_path: None,
                    render_tab: None,
                },
            );
        self.finish_pane_drop(ui, dock);
        self.finish_session_drop(ui, project, dock);
    }
}
