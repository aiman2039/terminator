use super::super::super::*;
use super::super::tabs::{
    group_terminal_ids, tab_attention_text, tab_label_width, workspace_tab_width,
};
use super::drop::StripDrop;
use super::tab::StripTabInput;
use super::{StripActions, StripFrame};
use crate::agent_presence::AttentionCounts;

/// Inputs for the strip row painted inside the horizontal layout.
struct StripRowInput<'a> {
    project: &'a str,
    strip_rect: egui::Rect,
    reveal: bool,
    workspace: &'a mut Workspace,
    actions: &'a mut StripActions,
}

/// Inputs for the scrolled run of strip tabs.
struct StripTabsInput<'a> {
    frame: StripFrame<'a>,
    workspace: &'a mut Workspace,
    tab_layout: &'a [(f32, AttentionCounts)],
    strip_rects: &'a mut Vec<(String, egui::Rect)>,
    actions: &'a mut StripActions,
}

impl App {
    pub(crate) fn workspace_bar(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        workspace: &mut Workspace,
    ) {
        self.cancel_pane_drag_on_escape(ui, workspace);
        let mut actions = StripActions::default();
        self.record_active_when_idle(workspace);
        let reveal = self.mark_workspace_visible(project, workspace);
        let previous_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
        let strip_rect = self.paint_strip_background(ui);
        ui.horizontal(|ui| {
            self.strip_row(
                ui,
                StripRowInput {
                    project,
                    strip_rect,
                    reveal,
                    workspace,
                    actions: &mut actions,
                },
            );
        });
        ui.painter().hline(
            strip_rect.x_range(),
            strip_rect.bottom(),
            egui::Stroke::new(1.0, appearance::color(&self.theme.border)),
        );
        ui.add_space(1.0);
        ui.spacing_mut().item_spacing = previous_spacing;
        self.apply_strip_actions(project, workspace, actions);
    }

    fn cancel_pane_drag_on_escape(&mut self, ui: &egui::Ui, workspace: &mut Workspace) {
        if self.drag.pane.is_some() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.revert_drop_preview(workspace);
            self.end_pane_drag();
            ui.ctx().stop_dragging();
        }
    }

    fn record_active_when_idle(&self, workspace: &mut Workspace) {
        if self.drag.pane.is_none() && self.drag.tab.is_none() && self.drag.preview_origin.is_none()
        {
            workspace.record_active();
        }
    }

    /// Remember which project tab is on screen. True when it just changed,
    /// so the active strip tab scrolls into view.
    fn mark_workspace_visible(&mut self, project: &str, workspace: &Workspace) -> bool {
        let current = (project.to_owned(), workspace.active.clone());
        let reveal = self.workspace_visible.as_ref() != Some(&current);
        self.workspace_visible = Some(current);
        reveal
    }

    fn paint_strip_background(&self, ui: &egui::Ui) -> egui::Rect {
        let strip_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 32.0));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "workspace-strip", strip_rect);
        ui.painter()
            .rect_filled(strip_rect, 0, appearance::color(&self.theme.surface));
        strip_rect
    }

    fn strip_row(&mut self, ui: &mut egui::Ui, input: StripRowInput<'_>) {
        let StripRowInput {
            project,
            strip_rect,
            reveal,
            workspace,
            actions,
        } = input;
        let tab_layout: Vec<(f32, AttentionCounts)> = workspace
            .tabs
            .iter()
            .map(|group| self.strip_tab_extent(ui, group))
            .collect();
        self.refresh_dock_back_targets(workspace);
        let (width, overflow) = Self::strip_scroll_width(ui, &tab_layout);
        let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("workspace-tabs", project)));
        let offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
            .map_or(0.0, |s| s.unclamped_offset().x);
        let direction = Self::scroll_tabs_left_button(ui, overflow, offset);
        Self::route_wheel_to_strip(ui, overflow, strip_rect);
        let mut strip_rects: Vec<(String, egui::Rect)> = Vec::with_capacity(workspace.tabs.len());
        let frame = StripFrame {
            project,
            strip_rect,
            reveal,
            drag_same_project: self.drag_same_project(project, workspace),
        };
        let scroll = egui::ScrollArea::horizontal()
            .id_salt(("workspace-tabs", project))
            .max_width(width)
            .auto_shrink([true, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.horizontal(|ui| {
                    self.strip_tabs(
                        ui,
                        StripTabsInput {
                            frame,
                            workspace: &mut *workspace,
                            tab_layout: &tab_layout,
                            strip_rects: &mut strip_rects,
                            actions: &mut *actions,
                        },
                    );
                });
            });
        if overflow {
            Self::scroll_tabs_right(ui, scroll, direction);
        }
        let plus_rect = self.strip_new_tab_buttons(ui);
        self.strip_recent_tabs_menu(ui, workspace, actions);
        let released = ui.input(|i| i.pointer.any_released());
        let mut drop = StripDrop {
            frame,
            plus_rect,
            strip_rects: &strip_rects,
            released,
            workspace,
            actions,
        };
        self.finish_strip_drags(ui, &mut drop);
        if self.drag.pane.is_some() {
            ui.ctx().request_repaint();
        }
        header_drag_space(ui);
    }

    /// Strip width for one top-level tab, and its aggregate attention.
    fn strip_tab_extent(
        &self,
        ui: &egui::Ui,
        group: &crate::workspace::WorkspaceTab,
    ) -> (f32, AttentionCounts) {
        let face = self.tab_face(Self::group_primary(group));
        let width = workspace_tab_width(tab_label_width(ui, &face.label))
            + if face.brand.is_some() { 6.0 } else { 0.0 };
        let attention = self.cached_tab_attention(&group_terminal_ids(&group.layout));
        (
            width
                + if tab_attention_text(attention).is_some() {
                    22.0
                } else {
                    0.0
                },
            attention,
        )
    }

    /// Sibling tabs for the pane menu's dock-back targets, refreshed every
    /// frame before the main dock paints (which checks the workspace out,
    /// so the menu cannot read it then).
    fn refresh_dock_back_targets(&mut self, workspace: &Workspace) {
        self.dock_back_targets = workspace
            .tabs
            .iter()
            .filter(|group| group.id != workspace.active)
            .map(|group| {
                let face = self.tab_face(Self::group_primary(group));
                (group.id.clone(), face.label, face.icon)
            })
            .collect();
    }

    /// Scroll viewport width for the tabs, and whether they overflow it.
    fn strip_scroll_width(ui: &egui::Ui, tab_layout: &[(f32, AttentionCounts)]) -> (f32, bool) {
        // Gap between tabs is a UI coordinate; the count can exceed the f32 mantissa.
        let gaps = tab_layout.len().saturating_sub(1) as f32;
        let content_width = tab_layout.iter().map(|(width, _)| *width).sum::<f32>() + gaps;
        // Reserve exactly the trailing `+` and menu buttons, and the two
        // scroll arrows on overflow, so the strip never clips a tab.
        let control = appearance::TOOLBAR_BUTTON + ui.spacing().item_spacing.x;
        let width = (ui.available_width() - 2.0 * control).max(40.0);
        let overflow = content_width > width;
        let width = (width - if overflow { 2.0 * control } else { 0.0 }).max(1.0);
        (width, overflow)
    }

    /// The left scroll arrow on overflow. Returns the scroll direction it
    /// requests: -1 when clicked, else 0.
    fn scroll_tabs_left_button(ui: &mut egui::Ui, overflow: bool, offset: f32) -> f32 {
        if !overflow {
            return 0.0;
        }
        let left = ui
            .add_enabled_ui(offset > 0.5, |ui| {
                appearance::sidebar_action(ui, "ChevronLeft", "Scroll tabs left")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "tabs-left", left.rect);
        if left.clicked() { -1.0 } else { 0.0 }
    }

    /// On overflow, a vertical wheel over the strip scrolls the tabs
    /// sideways unless a zoom modifier is held.
    fn route_wheel_to_strip(ui: &mut egui::Ui, overflow: bool, strip_rect: egui::Rect) {
        if overflow && ui.rect_contains_pointer(strip_rect) {
            ui.input_mut(|input| {
                if !input.modifiers.ctrl && !input.modifiers.command {
                    input.smooth_scroll_delta.x += input.smooth_scroll_delta.y;
                    input.smooth_scroll_delta.y = 0.0;
                }
            });
        }
    }

    /// Strip tabs are drop targets while a terminal pane is dragged, and
    /// drag handles for reordering the tabs themselves. Only match drags
    /// from this project.
    fn drag_same_project(&self, project: &str, workspace: &Workspace) -> bool {
        self.drag.pane.as_ref().is_some_and(|pane| {
            if let Some(source) = &self.drag.pane_source {
                return source.project == project && workspace.view_exists(source, pane);
            }
            match pane {
                Tab::Terminal(sid) => self
                    .state
                    .sessions
                    .iter()
                    .any(|s| &s.id == sid && s.project_id == project),
                _ => false,
            }
        })
    }

    fn strip_tabs(&mut self, ui: &mut egui::Ui, input: StripTabsInput<'_>) {
        let StripTabsInput {
            frame,
            workspace,
            tab_layout,
            strip_rects,
            actions,
        } = input;
        let tab_count = workspace.tabs.len();
        for (index, group) in workspace.tabs.iter().enumerate() {
            let Some(layout) = tab_layout.get(index).copied() else {
                continue;
            };
            self.strip_tab(
                ui,
                StripTabInput {
                    frame,
                    group,
                    index,
                    count: tab_count,
                    layout,
                    active_id: &mut workspace.active,
                    strip_rects: &mut *strip_rects,
                    actions: &mut *actions,
                },
            );
        }
    }

    /// The right scroll arrow, then the scroll either arrow requested.
    fn scroll_tabs_right<R>(
        ui: &mut egui::Ui,
        mut scroll: egui::scroll_area::ScrollAreaOutput<R>,
        mut direction: f32,
    ) {
        let max_offset = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
        let right = ui
            .add_enabled_ui(scroll.state.unclamped_offset().x < max_offset - 0.5, |ui| {
                appearance::sidebar_action(ui, "ChevronRight", "Scroll tabs right")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "tabs-right", right.rect);
        if right.clicked() {
            direction = 1.0;
        }
        if direction != 0.0 {
            let mut offset = scroll.state.unclamped_offset();
            offset.x = (offset.x + direction * scroll.inner_rect.width().max(1.0) * 0.8)
                .clamp(0.0, max_offset);
            super::super::write_scroll_offset(&mut scroll.state, offset);
            scroll.state.store(ui.ctx(), scroll.id);
            ui.ctx().request_repaint();
        }
    }

    /// The `+` button and new-tab menu. Returns the `+` rect, which is also
    /// a drop target for a fresh tab at the end of the strip.
    fn strip_new_tab_buttons(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let plus_response = appearance::sidebar_action(
            ui,
            "Plus",
            &format!(
                "{} — right-click for Browser or VNC",
                appearance::shortcut_tip("New tab", &self.shortcut_label(Action::NewTerminal))
            ),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "workspace-plus", plus_response.rect);
        let plus_rect = plus_response.rect;
        if plus_response.clicked() && self.drag.pane.is_none() {
            self.create(None);
        }
        appearance::context_menu(&plus_response, |ui| {
            self.workspace_new_tab_menu(ui);
        });
        appearance::icon_menu_button(ui, "ChevronDown", |ui| {
            self.workspace_new_tab_menu(ui);
        })
        .response
        .on_hover_text("New terminal, browser, or VNC tab");
        plus_rect
    }

    fn strip_recent_tabs_menu(
        &self,
        ui: &mut egui::Ui,
        workspace: &Workspace,
        actions: &mut StripActions,
    ) {
        let recent: Vec<(String, String, bool)> = workspace
            .recent_tabs
            .iter()
            .filter_map(|id| {
                workspace.tabs.iter().find(|tab| &tab.id == id).map(|tab| {
                    (
                        id.clone(),
                        self.tab_face(Self::group_primary(tab)).label,
                        tab.pinned,
                    )
                })
            })
            .collect();
        appearance::icon_menu_button(ui, "History", |ui| {
            ui.weak("Recent tabs");
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (id, label, pinned) in &recent {
                        let label = if *pinned {
                            format!("◆ {label}")
                        } else {
                            label.clone()
                        };
                        if ui
                            .selectable_label(workspace.active == *id, label)
                            .clicked()
                        {
                            actions.switch = Some(id.clone());
                            ui.close();
                        }
                    }
                });
        })
        .response
        .on_hover_text("Recent tabs");
    }

    fn apply_strip_actions(
        &mut self,
        project: &str,
        workspace: &mut Workspace,
        actions: StripActions,
    ) {
        let StripActions {
            switch,
            close,
            close_all,
            close_left,
            close_right,
            toggle_pin,
            add_at,
        } = actions;
        if let Some(id) = switch {
            self.switch_strip_tab(workspace, id);
        }
        if let Some(id) = close {
            self.begin_workspace_close_tabs(project, vec![id]);
        }
        if close_all {
            self.begin_workspace_close_tabs(project, workspace.unpinned_ids(workspace.ids()));
        }
        if let Some(id) = close_left {
            self.begin_workspace_close_tabs(
                project,
                workspace.unpinned_ids(workspace.ids_before(&id)),
            );
        }
        if let Some(id) = close_right {
            self.begin_workspace_close_tabs(
                project,
                workspace.unpinned_ids(workspace.ids_after(&id)),
            );
        }
        if let Some(id) = toggle_pin
            && let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.id == id)
        {
            tab.pinned = !tab.pinned;
        }
        if let Some(index) = add_at {
            self.create_workspace_tab(Some(index));
        }
    }

    fn switch_strip_tab(&mut self, workspace: &mut Workspace, id: String) {
        if workspace.active != id && self.rename_surface == RenameSurface::Pane {
            self.finish_rename(true);
        }
        workspace.active = id;
        workspace.record_active();
        self.hover_popup = None;
        self.hide_center_overlay();
    }
}
