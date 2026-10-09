use super::super::appearance;
use eframe::egui::{self};
use egui_dock::{DockArea, NodeIndex};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn workspace_sidebars(&mut self, ui: &mut egui::Ui) {
        if self.preferences.left_visible {
            let projects_response = egui::Panel::left("projects")
                .resizable(true)
                .default_size(225.0)
                .size_range(170.0..=420.0)
                .show(ui, |ui| {
                    self.agent_bar(ui);
                    ui.push_id("left-sidebar-content", |ui| {
                        if self.preferences.left_agents {
                            self.agents_view(ui);
                        } else {
                            self.projects(ui);
                        }
                    });
                });
            self.project_width = projects_response.response.rect.width();
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                "projects-sidebar",
                projects_response.response.rect,
            );
        }
        if self.preferences.visible {
            let response = egui::Panel::right("context")
                .resizable(true)
                .default_size(self.preferences.width)
                .size_range(220.0..=480.0)
                .show(ui, |ui| {
                    self.sidebar(ui);
                });
            self.preferences.width = response.response.rect.width().clamp(220.0, 480.0);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "context-sidebar", response.response.rect);
        }
    }

    /// Sidebars and the IDE terminal strip. Panel order is the layout:
    /// sidebars registered first run the full height and the strip stays in
    /// the center column; the strip registered first runs under both sidebars.
    pub(crate) fn place_ide_columns(&mut self, ui: &mut egui::Ui) {
        if self.preferences.ide_sidebars_full_height {
            self.workspace_sidebars(ui);
            self.ide_terminal_panel(ui);
        } else {
            self.ide_terminal_panel(ui);
            self.workspace_sidebars(ui);
        }
    }

    pub(crate) fn ide_terminal_panel(&mut self, ui: &mut egui::Ui) {
        if !(self.preferences.ide_mode && !self.preferences.ide_terminal_collapsed) {
            return;
        }
        let max_height = (ui.available_height() * 0.8).max(80.0);
        #[cfg_attr(not(feature = "test-support"), allow(unused_variables))]
        let shown = egui::Panel::bottom("ide-terminal")
            .resizable(true)
            .default_size(220.0)
            .size_range(80.0..=max_height)
            .show(ui, |ui| {
                self.ide_terminal_strip(ui);
            });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "ide-terminal-panel", shown.response.rect);
    }

    pub(crate) fn toggle_left_sidebar(&mut self) {
        self.preferences.left_visible = !self.preferences.left_visible;
    }

    pub(crate) fn toggle_right_sidebar(&mut self) {
        self.preferences.visible = !self.preferences.visible;
    }

    pub(crate) fn toggle_ide_sidebar_height(&mut self) {
        self.preferences.ide_sidebars_full_height = !self.preferences.ide_sidebars_full_height;
    }

    pub(crate) fn toggle_ide_mode(&mut self) {
        self.preferences.ide_mode = !self.preferences.ide_mode;
        if self.preferences.ide_mode {
            // Sidebar visibility is user-controlled in both modes.
            // Switching modes must not overwrite the saved flags.
            // The strip opens so shells that just moved are on screen.
            self.preferences.ide_terminal_collapsed = false;
            self.move_shells_to_strip();
            if let (Some(project), Some(sid)) = (self.selected.clone(), self.active_session.clone())
                && self.is_strip_session(&project, &sid)
            {
                self.activate_strip_session(&project, &sid);
            }
        } else {
            self.move_shells_to_main();
            self.resync_active_from_dock();
        }
        // The relocation is not a click. Recording the docks now keeps the
        // next focus sync from treating it as one.
        self.note_focus_baselines();
        self.save_layouts();
    }

    /// Right end of the bottom status strip. One right-to-left block so the
    /// terminal button keeps the far-right corner, the sidebar-height toggle
    /// sits to its left, and the resource readout sits left of that.
    pub(crate) fn status_right_end(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.preferences.ide_mode {
                // Same icon collapsed or expanded: clicking toggles the
                // strip, so the corner control never changes shape.
                let collapsed = self.preferences.ide_terminal_collapsed;
                let toggle = appearance::sidebar_action(
                    ui,
                    "PanelBottomClose",
                    if collapsed {
                        "Show IDE terminal strip"
                    } else {
                        "Hide terminal strip"
                    },
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "status-terminal-toggle", toggle.rect);
                if toggle.clicked() {
                    self.preferences.ide_terminal_collapsed = !collapsed;
                    if !collapsed {
                        self.resync_active_from_dock();
                    }
                }
                let full_height = self.preferences.ide_sidebars_full_height;
                let height = appearance::selectable_icon(
                    ui,
                    "Columns3",
                    if full_height {
                        "Terminal full width"
                    } else {
                        "Sidebars full height"
                    },
                    full_height,
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "status-sidebar-height", height.rect);
                if height.clicked() {
                    self.toggle_ide_sidebar_height();
                }
                ui.separator();
            }
            self.app_resource_status(ui);
            if self.preferences.ide_mode {
                ui.separator();
            }
        });
    }

    /// True when keyboard focus belongs to a strip terminal: IDE mode with a
    /// visible strip and a live strip session active. Pane-relative actions
    /// (splits) follow this; workspace-level "new tab" stays in the main dock.
    pub(crate) fn strip_focused(&self) -> bool {
        self.ide_strip_visible()
            && self.active_session.as_deref().is_some_and(|sid| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| s.id == sid)
                    .is_some_and(|s| {
                        s.lifecycle.live() && self.is_strip_session(&s.project_id, sid)
                    })
            })
    }

    /// Insert a tab into the strip dock, splitting the focused leaf when asked.
    /// Mirrors [`Self::insert`], which serves the main dock.
    pub(crate) fn insert_strip(&mut self, project: &str, tab: Tab, split: Option<&str>) {
        let dock = self
            .preferences
            .ide_strip_docks
            .0
            .entry(project.into())
            .or_insert_with(|| egui_dock::DockState::new(vec![]));
        if let Some(path) = dock.find_tab(&tab) {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
            return;
        }
        if let Some(direction) = split {
            let tree = dock.main_surface_mut();
            if !tree.is_empty() {
                let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
                let result = match direction {
                    "left" => tree.split_left(node, 0.5, vec![tab]),
                    "up" => tree.split_above(node, 0.5, vec![tab]),
                    "down" => tree.split_below(node, 0.5, vec![tab]),
                    _ => tree.split_right(node, 0.5, vec![tab]),
                };
                tree.set_focused_node(result[1]);
                return;
            }
        }
        dock.push_to_focused_leaf(tab);
    }

    /// Strip-dock counterpart of [`Self::editor_target`]: anchor a creation on
    /// the origin pane's leaf, else the strip's focused leaf.
    pub(crate) fn strip_target(
        &self,
        project: &str,
        origin: Option<&Tab>,
        split: Option<&str>,
    ) -> After {
        let mut anchors = Vec::new();
        if let Some(dock) = self.preferences.ide_strip_docks.0.get(project) {
            let path = origin
                .and_then(|tab| dock.find_tab(tab).map(egui_dock::TabPath::node_path))
                .or_else(|| {
                    dock.main_surface()
                        .focused_leaf()
                        .map(|node| egui_dock::NodePath {
                            surface: egui_dock::SurfaceIndex::main(),
                            node,
                        })
                });
            if let Some(path) = path
                && let Ok(leaf) = dock.leaf(path)
            {
                if let Some(tab) = leaf.tabs.get(leaf.active.0) {
                    anchors.push(tab.clone());
                }
                anchors.extend(
                    leaf.tabs
                        .iter()
                        .filter(|tab| !anchors.contains(tab))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
        }
        match split {
            Some(split) => After::StripAt(anchors, Some(split.into())),
            None => After::Strip,
        }
    }

    /// Open a shell in the strip dock, splitting the focused strip leaf when asked.
    pub(crate) fn create_strip_split(&mut self, split: Option<&str>) {
        if self
            .selected
            .as_ref()
            .is_some_and(|project| self.missing_projects.contains(project))
        {
            return;
        }
        self.hide_center_overlay();
        if let Some(project) = self.selected.clone() {
            let after = self.strip_target(&project, None, split);
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project,
                    cwd: self.cwd(),
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
                after,
            ));
        }
    }

    /// Open a shell owned by the IDE strip (never inserted as a dock tab).
    pub(crate) fn create_strip(&mut self) {
        if self
            .selected
            .as_ref()
            .is_some_and(|project| self.missing_projects.contains(project))
        {
            return;
        }
        self.hide_center_overlay();
        if let Some(project) = self.selected.clone() {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project,
                    cwd: None,
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
                After::Strip,
            ));
        }
    }

    /// Re-derive the active session from the dock. Leaving IDE mode or hiding
    /// the strip orphans a strip-owned `active_session`; dock focus keeps
    /// working only if it points at a visible terminal again.
    pub(crate) fn resync_active_from_dock(&mut self) {
        let keep = match self
            .active_session
            .as_deref()
            .and_then(|sid| self.state.sessions.iter().find(|s| s.id == sid))
        {
            Some(s) => !(s.lifecycle.live() && self.is_strip_session(&s.project_id, &s.id)),
            None => self.active_session.is_none(),
        };
        if keep {
            return;
        }
        self.active_session = self
            .selected
            .as_ref()
            .and_then(|project| self.layouts.get(project))
            .and_then(|dock| dock.active_pane())
            .and_then(|tab| match tab {
                Tab::Terminal(sid) => Some(sid.clone()),
                _ => None,
            });
    }

    /// Heal an `active_session` cleared by a tab close: prefer the strip's
    /// live selection while the strip is visible, else the dock's focused
    /// terminal. Runs after [`Self::sync_active_session`], which only follows
    /// dock focus changes so a strip click is never clobbered.
    ///
    /// A focused image, browser, diff, player, or editor is a real selection.
    /// That `None` must stay `None`. Closing the terminal that was focused
    /// afterwards still heals.
    pub(crate) fn restore_cleared_focus(&mut self, dock: &Workspace) {
        if self.active_session.is_some() || self.non_terminal_selected {
            return;
        }
        if self.ide_strip_visible()
            && let Some(project) = self.selected.clone()
            && let Some(next) = self.strip_first_live(&project)
        {
            self.activate_strip_session(&project, &next);
            return;
        }
        if let Some(Tab::Terminal(sid)) = dock.active_pane() {
            self.active_session = Some(sid.clone());
        }
    }

    pub(crate) fn ide_terminal_strip(&mut self, ui: &mut egui::Ui) {
        // A move queued last frame lands before this dock is checked out.
        self.drain_strip_move();
        // Keep the resizable panel at its stored height even when the strip has
        // nothing to show. Otherwise the frame shrinks to its content and the
        // separator jumps down for whichever project you switch away from.
        ui.set_min_height(ui.available_height());
        let Some(project) = self.selected.clone() else {
            ui.weak("Select a project to use the terminal strip.");
            return;
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "ide-terminal-strip", ui.max_rect());
        let mut strip = self
            .preferences
            .ide_strip_docks
            .0
            .remove(&project)
            .unwrap_or_else(|| egui_dock::DockState::new(vec![]));
        if strip.iter_all_tabs().next().is_none() {
            let remaining = ui.available_size();
            ui.allocate_ui_with_layout(
                remaining,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.add_space(((remaining.y - 120.0) * 0.5).max(6.0));
                    ui.add(
                        egui::Image::new(crate::icons::source("Terminal"))
                            .tint(appearance::color(&self.theme.secondary))
                            .fit_to_exact_size(egui::vec2(24.0, 24.0)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("No terminal open")
                            .color(appearance::color(&self.theme.secondary)),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Start one to dock it here.")
                                .size(11.0)
                                .color(appearance::color(&self.theme.secondary)),
                        )
                        .selectable(false),
                    );
                    ui.add_space(10.0);
                    if ui
                        .add(egui::Button::image_and_text(
                            egui::Image::new(crate::icons::source("Terminal"))
                                .tint(appearance::ICON_COLOR)
                                .fit_to_exact_size(egui::vec2(13.0, 13.0)),
                            "Open terminal",
                        ))
                        .clicked()
                    {
                        self.create_strip();
                    }
                },
            );
        } else {
            let style = self.dock_style(ui);
            self.refresh_strip_pane_maps(&strip);
            DockArea::new(&mut strip)
                // Distinct area id: an in-strip reorder stays here. Dragging
                // the tab out of this rect promotes it to a pane drag, which
                // the main pane accepts.
                .id(egui::Id::new("ide-strip-dock"))
                .style(style)
                .show_add_buttons(true)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(
                    ui,
                    &mut Viewer {
                        app: self,
                        strip: true,
                        project: Some(project.clone()),
                        // Strip tabs carry no top-level id: closes from
                        // here fall back to legacy docked removal.
                        tab: None,
                        window: None,
                        render_path: None,
                    },
                );
            self.apply_add_strip_tab(&project, &mut strip);
            self.apply_focus_strip_tab(&mut strip);
        }
        let strip_rect = ui.max_rect();
        self.preferences.ide_strip_docks.0.insert(project, strip);
        // The context menu queues during `show`, while this dock was checked
        // out. Apply that move now that membership is visible again.
        self.drain_strip_move();
        // Same frame as a release: later panels see a promoted strip drag,
        // and a main-pane shell released here moves into the dock just inserted.
        self.track_strip_tab_drag(ui, strip_rect);
        self.take_main_drop_on_strip(ui, strip_rect);
    }
}
