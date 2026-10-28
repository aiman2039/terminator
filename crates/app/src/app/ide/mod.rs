//! IDE layout: sidebars beside the tool stripes, the bottom terminal
//! strip panel, and the layout toggles in the status bar.

use crate::*;

mod stripe;
mod terminal_strip;

/// A stripe entry: the native Projects list, or one right-sidebar tool.
/// Either stripe can host any of them; the side decides the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StripeTool {
    Projects,
    Tool(SidebarTool),
}

impl App {
    pub(crate) fn workspace_sidebars(&mut self, ui: &mut egui::Ui) {
        // Stripes are the tool-window access point in both layouts; IDE
        // mode only adds the bottom terminal strip on top of them.
        self.tool_stripe(ui, false);
        self.tool_stripe(ui, true);
        if self.preferences.left_visible {
            let projects_response = egui::Panel::left("projects")
                .frame(appearance::panel_frame(&self.theme))
                .resizable(true)
                .default_size(225.0)
                .size_range(170.0..=420.0)
                .show(ui, |ui| {
                    self.agent_bar(ui);
                    ui.push_id("left-sidebar-content", |ui| {
                        if let Some(tool) = self.preferences.left_tool {
                            self.sidebar_tool(tool, ui);
                        } else if self.preferences.left_agents {
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
                .frame(appearance::panel_frame(&self.theme))
                .resizable(true)
                .default_size(self.preferences.width)
                .size_range(220.0..=480.0)
                .show(ui, |ui| {
                    if self.preferences.right_projects {
                        self.agent_bar(ui);
                        self.projects(ui);
                    } else {
                        self.sidebar(ui);
                    }
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
        #[cfg_attr(
            not(feature = "test-support"),
            expect(unused_variables, reason = "only fixtures record the panel rect")
        )]
        let shown = egui::Panel::bottom("ide-terminal")
            .frame(appearance::panel_frame(&self.theme))
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
                let height = appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "Columns3",
                    tip: if full_height {
                        "Terminal full width"
                    } else {
                        "Sidebars full height"
                    },
                    selected: full_height,
                });
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
}
