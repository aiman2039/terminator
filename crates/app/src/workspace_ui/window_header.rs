use super::super::*;
#[cfg(not(target_os = "macos"))]
use super::header::paint_window_controls;
use super::header::{
    HEADER_ACTIONS, HEADER_GAP, HEADER_MENU_SLOT, HEADER_SLOT, HeaderAction, HeaderActionView,
    HeaderIconButton, HeaderToolBounds, PROJECT_HEADER_GAP, PROJECT_HEADER_SLOT,
    ProjectHeaderChrome, header_action_view, header_icon_button, header_tool_bounds,
    header_visible_count, project_header_chrome, project_title_width,
};
impl App {
    pub(super) fn show_git_sidebar(&mut self) {
        self.preferences.visible = true;
        self.preferences.tool = SidebarTool::Git;
    }

    pub(super) fn action_tip(&self, label: &str, action: &str) -> String {
        let keys = self.shortcut_label(action);
        if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        }
    }

    pub(super) fn header_left_width(&self, total: f32) -> f32 {
        let cap = (total - 300.0).max(80.0);
        if self.preferences.left_visible {
            self.project_width.min(cap)
        } else if cfg!(target_os = "macos") {
            200.0_f32.min(cap)
        } else {
            168.0_f32.min(cap)
        }
    }

    pub(super) fn sidebar_toggle(&mut self, ui: &mut egui::Ui, right: bool) {
        let (action, icon, label) = if right {
            ("toggle_right_sidebar", "PanelRight", "Toggle right sidebar")
        } else {
            ("toggle_left_sidebar", "PanelLeft", "Toggle sidebar")
        };
        let keys = self.shortcut_label(action);
        let tip = if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        };
        let response = appearance::framed_icon(ui, icon, &tip);
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            if right {
                "toggle-right-sidebar"
            } else {
                "toggle-left-sidebar"
            },
            response.rect,
        );
        if response.clicked() {
            if right {
                self.toggle_right_sidebar();
            } else {
                self.toggle_left_sidebar();
            }
        }
    }

    pub(super) fn project_header_name(&self) -> String {
        self.selected_project()
            .map_or_else(|| "Terminator".to_string(), |project| project.name.clone())
    }

    /// macOS traffic lights and the Linux close, minimize, and maximize
    /// buttons share this width. Wider Linux buttons steal the project name
    /// and fold Player and Agents into the menu at the default sidebar width.
    const WINDOW_CONTROL_RESERVE: f32 = 72.0;

    pub(super) fn window_controls(&mut self, ui: &mut egui::Ui) {
        let native = ui
            .input(|i| i.viewport().native_pixels_per_point)
            .unwrap_or(ui.ctx().pixels_per_point());
        let width = Self::WINDOW_CONTROL_RESERVE * native / ui.ctx().pixels_per_point();
        let row = ui.cursor();
        let height = 28.0_f32.min(row.height());
        // macOS without the fixture harness consumes neither use below;
        // the underscore keeps plain builds warning-free.
        let _rect = egui::Rect::from_min_size(
            egui::pos2(row.left(), row.center().y - height * 0.5),
            egui::vec2(width, height),
        );
        ui.add_space(width);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "window-controls", _rect);
        #[cfg(not(target_os = "macos"))]
        paint_window_controls(ui, _rect);
    }

    /// Left-align the project name; spare width is drag space between the
    /// name and Player, Agents, and hide, which stay packed on the right.
    /// A narrow sidebar folds Player and Agents into the menu.
    pub(super) fn project_header_cluster(&mut self, ui: &mut egui::Ui) {
        self.note_agent_bar_badge(ui);
        let name = self.project_header_name();
        let natural = project_title_width(ui, &name);
        let chrome = project_header_chrome(ui.available_width(), natural);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = PROJECT_HEADER_GAP;
            match chrome {
                ProjectHeaderChrome::Icons { name: width } => {
                    self.project_header_title(ui, &name, width);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = PROJECT_HEADER_GAP;
                        self.sidebar_toggle(ui, false);
                        self.header_agents_button(ui);
                        self.header_player_button(ui);
                        header_drag_space(ui);
                    });
                }
                ProjectHeaderChrome::Menu { name: width, hide } => {
                    self.project_header_title(ui, &name, width);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = PROJECT_HEADER_GAP;
                        if !hide {
                            self.sidebar_toggle(ui, false);
                        }
                        self.project_header_menu(ui, hide);
                        header_drag_space(ui);
                    });
                }
            }
        });
    }

    pub(super) fn project_header_title(&mut self, ui: &mut egui::Ui, name: &str, width: f32) {
        if width <= 0.0 || name.is_empty() {
            return;
        }
        let response = ui.add_sized(
            [width, PROJECT_HEADER_SLOT],
            egui::Label::new(RichText::new(name).strong())
                .truncate()
                .sense(egui::Sense::drag()),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-header-name", response.rect);
        if response.drag_started() {
            begin_native_window_gesture(ui.ctx(), egui::ViewportCommand::StartDrag);
        }
    }

    pub(super) fn project_header_menu(&mut self, ui: &mut egui::Ui, hide: bool) {
        let response = appearance::framed_icon(ui, "Menu", "More");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-header-menu", response.rect);
        egui::Popup::menu(&response)
            .style(appearance::menu_style)
            .show(|ui| {
                if self.header_player_menu(ui) {
                    ui.close();
                }
                if self.header_agents_menu(ui) {
                    ui.close();
                }
                if hide && self.header_sidebar_menu(ui) {
                    ui.close();
                }
            });
    }

    pub(super) fn header_sidebar_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let label = if self.preferences.left_visible {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        let shortcut = self.shortcut_label("toggle_left_sidebar");
        let clicked = appearance::menu_item(ui, label, "PanelLeft", &shortcut).clicked();
        if clicked {
            self.toggle_left_sidebar();
        }
        clicked
    }

    /// Single drag-band row: traffic lights, project label, and tools.
    /// The tab strip is a separate center panel below the sidebars' top
    /// edge so the sidebars run full height.
    pub(crate) fn window_header(&mut self, ui: &mut egui::Ui) {
        self.window_header_row(ui);
    }

    /// Top header row: traffic lights, project label, and tools. The middle
    /// is window drag space; tabs live in [`Self::window_header_tabs`].
    pub(super) fn window_header_row(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let left = self.header_left_width(rect.width());
        let right = self.preferences.width.min(rect.width() - left - 100.0);
        let left_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + left, rect.bottom()));
        let tools_rect =
            egui::Rect::from_min_max(egui::pos2(rect.right() - right, rect.top()), rect.max);
        let tabs_rect = egui::Rect::from_min_max(
            egui::pos2(left_rect.right(), rect.top() + 4.0),
            egui::pos2(tools_rect.left(), rect.bottom() - 4.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect.shrink2(egui::vec2(4.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                self.window_controls(ui);
                self.project_header_cluster(ui);
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
            ui.set_clip_rect(tabs_rect);
            header_drag_space(ui);
        });
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(tools_rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| self.header_tools(ui),
        );
    }

    /// Center-only tab strip below the native drag band, where egui owns
    /// every gesture so tabs can be dragged to reorder. Shown as a top
    /// panel after the sidebars so the strip sits beside them, not above.
    pub(crate) fn window_header_tabs(&mut self, ui: &mut egui::Ui) {
        ui.set_clip_rect(ui.max_rect());
        if let Some(project) = self.selected.clone() {
            let mut workspace = self
                .layouts
                .remove(&project)
                .unwrap_or_else(Workspace::empty);
            self.workspace_bar(ui, &project, &mut workspace);
            self.layouts.insert(project, workspace);
        } else {
            header_drag_space(ui);
        }
        // Deferred native closes (`:q`, `:wq`, `:qa` from the file view)
        // run here: workspaces are checked back in, so the tabs resolve
        // again. Unconditional: with no project selected there is no
        // checkout, so floating close requests must still drain instead
        // of stalling with their windows open.
        self.drain_pending_native_close();
    }

    pub(super) fn header_tools(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = HEADER_GAP;
        let HeaderToolBounds { budget, toggle } = header_tool_bounds(ui);
        ui.set_max_width(budget);
        let visible = header_visible_count(budget, HEADER_ACTIONS.len());
        self.paint_visible_header_actions(ui, visible);
        self.paint_header_overflow(ui, visible);
        header_drag_space(ui);
        self.paint_header_toggle(ui, toggle);
    }

    pub(super) fn paint_visible_header_actions(&mut self, ui: &mut egui::Ui, visible: usize) {
        for action in HEADER_ACTIONS.iter().take(visible).copied() {
            self.paint_header_action(ui, action);
        }
    }

    pub(super) fn paint_header_overflow(&mut self, ui: &mut egui::Ui, visible: usize) {
        if visible >= HEADER_ACTIONS.len() {
            return;
        }
        let Some(hidden) = HEADER_ACTIONS.get(visible..) else {
            return;
        };
        let response = header_icon_button(
            ui,
            HeaderIconButton {
                icon: "Menu",
                tip: "More",
                width: HEADER_MENU_SLOT,
            },
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "header-overflow", response.rect);
        egui::Popup::menu(&response)
            .style(appearance::menu_style)
            .show(|ui| {
                for action in hidden {
                    if self.header_menu_choice(ui, *action) {
                        ui.close();
                    }
                }
            });
    }

    pub(super) fn paint_header_toggle(&mut self, ui: &mut egui::Ui, toggle: egui::Rect) {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(toggle)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| self.sidebar_toggle(ui, true),
        );
    }

    pub(super) fn paint_header_action(&mut self, ui: &mut egui::Ui, action: HeaderAction) {
        let view = header_action_view(action);
        let response = match action {
            HeaderAction::Tool(tool) => self.header_tool_button(ui, tool, &view),
            HeaderAction::IdeMode | HeaderAction::Settings | HeaderAction::Palette => {
                header_icon_button(
                    ui,
                    HeaderIconButton {
                        icon: view.icon,
                        tip: &self.header_tip(action),
                        width: HEADER_SLOT,
                    },
                )
            }
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), view.target, response.rect);
        if response.clicked() {
            self.run_header_action(action);
        }
    }

    pub(super) fn header_tool_button(
        &mut self,
        ui: &mut egui::Ui,
        tool: SidebarTool,
        view: &HeaderActionView,
    ) -> egui::Response {
        let response =
            appearance::tool_button(ui, tool, view.label, self.header_tool_selected(tool));
        if tool == SidebarTool::Explorer {
            response.on_hover_text(self.explorer_tooltip())
        } else {
            response
        }
    }

    pub(super) fn header_menu_choice(&mut self, ui: &mut egui::Ui, action: HeaderAction) -> bool {
        let view = header_action_view(action);
        let mark = self.header_menu_mark(action);
        let response = appearance::menu_item(ui, view.label, view.icon, &mark);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), view.target, response.rect);
        if response.clicked() {
            self.run_header_action(action);
            true
        } else {
            false
        }
    }

    pub(super) fn header_tip(&self, action: HeaderAction) -> String {
        let label = header_action_view(action).label;
        let keys = self.header_menu_mark(action);
        if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        }
    }

    pub(super) fn header_menu_mark(&self, action: HeaderAction) -> String {
        match action {
            HeaderAction::Tool(tool) if self.header_tool_selected(tool) => "✓".into(),
            HeaderAction::IdeMode if self.preferences.ide_mode => "✓".into(),
            HeaderAction::Settings => self.shortcut_label("open_settings"),
            HeaderAction::Palette => self.shortcut_label("open_palette"),
            HeaderAction::IdeMode => self.shortcut_label("toggle_ide_mode"),
            HeaderAction::Tool(_) => String::new(),
        }
    }

    pub(super) fn header_tool_selected(&self, tool: SidebarTool) -> bool {
        self.preferences.visible && self.preferences.tool == tool
    }

    pub(super) fn run_header_action(&mut self, action: HeaderAction) {
        match action {
            HeaderAction::Tool(tool) => self.preferences.toggle(tool),
            HeaderAction::IdeMode => self.toggle_ide_mode(),
            HeaderAction::Settings => self.open_settings(),
            HeaderAction::Palette => self.open_command_palette(),
        }
    }
}
