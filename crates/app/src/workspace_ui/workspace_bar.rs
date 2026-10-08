use super::super::*;
use super::tab_menu::WorkspaceTabMenuSpec;
use super::tabs::{
    TabFace, TabLeading, group_terminal_ids, tab_attention_text, tab_label_width, tab_tooltip,
    workspace_tab_width,
};
impl App {
    /// Workspace strip tab face: the focused terminal's label, kind icon,
    /// stable agent brand, and hook lifecycle status, all from the shared
    /// agent presentation model.
    pub(crate) fn tab_face(&self, primary: Option<&Tab>) -> TabFace {
        match primary {
            Some(Tab::Terminal(sid)) => self
                .state
                .sessions
                .iter()
                .find(|s| &s.id == sid)
                .map(|s| {
                    let presented = self.present_session(sid);
                    TabFace {
                        label: s.label.clone(),
                        icon: if s.kind == SessionKind::Editor {
                            "FileCode"
                        } else {
                            "Terminal"
                        },
                        sid: Some(sid.clone()),
                        brand: presented.brand_icon,
                        status: presented.lifecycle.map(|lifecycle| {
                            (
                                lifecycle,
                                presented.status_icon,
                                crate::sidebar_ui::state_color(lifecycle, &self.theme),
                            )
                        }),
                    }
                })
                .unwrap_or(TabFace {
                    label: "Terminal".into(),
                    icon: "Terminal",
                    sid: None,
                    brand: None,
                    status: None,
                }),
            Some(Tab::Diff { path, .. } | Tab::Image { path }) => TabFace {
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                icon: if matches!(primary, Some(Tab::Image { .. })) {
                    "FileImage"
                } else {
                    "FileDiff"
                },
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Browser { target, .. }) => TabFace {
                label: target.title(),
                icon: "FileCode",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::NativeEditor { path }) => TabFace {
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                icon: "FileCode",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Player) => TabFace {
                label: "Player".into(),
                icon: "FileMusic",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::CommitLog { .. }) => TabFace {
                label: "Commit Log".into(),
                icon: "FileDiff",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Blame { path, .. }) => TabFace {
                label: format!(
                    "Blame {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                icon: "FileDiff",
                sid: None,
                brand: None,
                status: None,
            },
            None => TabFace {
                label: "Workspace".into(),
                icon: "Terminal",
                sid: None,
                brand: None,
                status: None,
            },
        }
    }

    /// Resolved leading icons for a terminal tab. Process inspection supplies
    /// the brand before a hook exists. Hook lifecycle then replaces the
    /// session-kind glyph (spinning while running). Unknown sessions resolve
    /// to no icons.
    pub(super) fn tab_leading(&self, sid: &str) -> TabLeading {
        let presented = self.present_session(sid);
        let kind = self.state.sessions.iter().find(|s| s.id == sid).map(|s| {
            if s.kind == SessionKind::Editor {
                "FileCode"
            } else {
                "Terminal"
            }
        });
        let status = presented.lifecycle.map(|lifecycle| {
            (
                presented.status_icon,
                crate::sidebar_ui::state_color(lifecycle, &self.theme),
                lifecycle == AgentState::Running,
            )
        });
        TabLeading {
            brand: presented.brand_icon,
            kind: status.is_none().then_some(kind).flatten(),
            status,
        }
    }

    pub(crate) fn workspace_bar(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        workspace: &mut Workspace,
    ) {
        let mut switch = None;
        let mut close = None;
        let mut close_all = false;
        let mut close_left = None;
        let mut close_right = None;
        let mut add_at = None;
        let current = (project.to_owned(), workspace.active.clone());
        let reveal = self.workspace_visible.as_ref() != Some(&current);
        self.workspace_visible = Some(current);
        let previous_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
        let strip_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 32.0));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "workspace-strip", strip_rect);
        ui.painter()
            .rect_filled(strip_rect, 0, appearance::color(&self.theme.surface));
        ui.horizontal(|ui| {
            let tab_layout: Vec<(f32, crate::agent_presence::AttentionCounts)> = workspace
                .tabs
                .iter()
                .map(|group| {
                    let primary = group
                        .primary
                        .as_ref()
                        .filter(|tab| group.layout.find_tab(tab).is_some())
                        .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab));
                    let face = self.tab_face(primary);
                    let width = workspace_tab_width(tab_label_width(ui, &face.label));
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
                })
                .collect();
            // Sibling tabs for the pane menu's dock-back targets, refreshed
            // every frame before the main dock paints (which checks the
            // workspace out, so the menu cannot read it then).
            self.dock_back_targets = workspace
                .tabs
                .iter()
                .filter(|group| group.id != workspace.active)
                .map(|group| {
                    let primary = group
                        .primary
                        .as_ref()
                        .filter(|tab| group.layout.find_tab(tab).is_some())
                        .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab));
                    let face = self.tab_face(primary);
                    (group.id.clone(), face.label, face.icon)
                })
                .collect();
            // Gap between tabs is a UI coordinate; the count can exceed the f32 mantissa.
            #[allow(clippy::cast_precision_loss)]
            let gaps = tab_layout.len().saturating_sub(1) as f32;
            let content_width = tab_layout.iter().map(|(width, _)| *width).sum::<f32>() + gaps;
            let width = (ui.available_width() - 38.0).max(40.0);
            let overflow = content_width > width;
            let width = (width - if overflow { 58.0 } else { 0.0 }).max(1.0);
            let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("workspace-tabs", project)));
            let offset =
                egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |s| s.offset.x);
            let mut direction = 0.0;
            if overflow {
                let left = ui
                    .add_enabled(
                        offset > 0.5,
                        egui::Button::new("‹").min_size(egui::vec2(27.0, 30.0)),
                    )
                    .on_hover_text("Scroll tabs left");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "tabs-left", left.rect);
                if left.clicked() {
                    direction = -1.0;
                }
            }
            if overflow && ui.rect_contains_pointer(strip_rect) {
                ui.input_mut(|input| {
                    if !input.modifiers.ctrl && !input.modifiers.command {
                        input.smooth_scroll_delta.x += input.smooth_scroll_delta.y;
                        input.smooth_scroll_delta.y = 0.0;
                    }
                });
            }
            let mut strip_rects: Vec<(String, egui::Rect)> =
                Vec::with_capacity(workspace.tabs.len());
            // Strip tabs are drop targets while a terminal pane is dragged,
            // and drag handles for reordering the tabs themselves. Only
            // match drags from this project.
            let drag_same_project = self.pane_drag.as_ref().is_some_and(|pane| match pane {
                Tab::Terminal(sid) => self
                    .state
                    .sessions
                    .iter()
                    .any(|s| &s.id == sid && s.project_id == project),
                _ => false,
            });
            let mut scroll = egui::ScrollArea::horizontal()
                .id_salt(("workspace-tabs", project))
                .max_width(width)
                .auto_shrink([true, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.horizontal(|ui| {
                        let tab_count = workspace.tabs.len();
                        for (index, group) in workspace.tabs.iter().enumerate() {
                            let primary = group
                                .primary
                                .as_ref()
                                .filter(|tab| group.layout.find_tab(tab).is_some())
                                .or_else(|| {
                                    group.layout.iter_all_tabs().next().map(|(_, tab)| tab)
                                });
                            let face = self.tab_face(primary);
                            let Some((width, attention)) = tab_layout.get(index).copied() else {
                                continue;
                            };
                            let badge = tab_attention_text(attention);
                            let active = workspace.active == group.id;
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, 32.0),
                                egui::Sense::click_and_drag(),
                            );
                            strip_rects.push((group.id.clone(), rect));
                            // Dragging a strip tab reorders the top-level
                            // tabs; pane drags keep their own payload.
                            if response.drag_started()
                                && self.pane_drag.is_none()
                                && self.tab_drag.is_none()
                            {
                                self.tab_drag = Some(group.id.clone());
                            }
                            // Destination preview: hovering a strip tab while
                            // dragging a terminal shows that tab's splits so
                            // the drop can target a specific leaf. Switching
                            // back over the origin tab restores it.
                            let hovering_tab = drag_same_project
                                && ui.input(|i| i.pointer.any_down())
                                && ui
                                    .input(|i| i.pointer.interact_pos())
                                    .is_some_and(|pos| rect.contains(pos));
                            if hovering_tab {
                                if self
                                    .drop_preview_origin
                                    .as_ref()
                                    .is_some_and(|(p, g)| p == project && g == &group.id)
                                {
                                    self.drop_preview_origin = None;
                                } else if self.drop_preview_origin.is_none()
                                    && workspace.active != group.id
                                {
                                    self.drop_preview_origin =
                                        Some((project.to_owned(), workspace.active.clone()));
                                }
                                workspace.active.clone_from(&group.id);
                                ui.painter().rect_stroke(
                                    rect,
                                    0,
                                    egui::Stroke::new(2.0, appearance::color(&self.theme.accent)),
                                    egui::StrokeKind::Inside,
                                );
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                                ui.ctx().request_repaint();
                            }
                            if active && reveal {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if active || response.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    0,
                                    if active {
                                        appearance::color(&self.theme.window)
                                    } else {
                                        appearance::color(&self.theme.hover)
                                    },
                                );
                            }
                            let tint = appearance::color(if active {
                                &self.theme.text
                            } else {
                                &self.theme.secondary
                            });
                            // Focused terminal identity: stable brand glyph
                            // beside the hook lifecycle status.
                            if let Some(brand) = face.brand {
                                appearance::paint_status_icon(
                                    ui,
                                    egui::Rect::from_center_size(
                                        egui::pos2(rect.left() + 10.0, rect.center().y),
                                        egui::vec2(13.0, 13.0),
                                    ),
                                    brand,
                                    appearance::ICON_COLOR,
                                    false,
                                );
                            }
                            let status_center = if face.brand.is_some() {
                                rect.left() + 24.0
                            } else {
                                rect.left() + 16.0
                            };
                            let status_size = if face.brand.is_some() { 13.0 } else { 16.0 };
                            let icon_rect = egui::Rect::from_center_size(
                                egui::pos2(status_center, rect.center().y),
                                egui::vec2(status_size, status_size),
                            );
                            if let Some((state, status_icon, tint)) = face.status {
                                appearance::paint_status_icon(
                                    ui,
                                    icon_rect,
                                    status_icon,
                                    tint,
                                    state == AgentState::Running,
                                );
                            } else {
                                egui::Image::new(icons::source(face.icon))
                                    .tint(appearance::ICON_COLOR)
                                    .paint_at(ui, icon_rect);
                            }
                            let label_x = if face.brand.is_some() { 36.0 } else { 30.0 };
                            let badge_reserve = if badge.is_some() { 22.0 } else { 0.0 };
                            let editing = face
                                .sid
                                .as_ref()
                                .is_some_and(|sid| self.renaming(sid, RenameSurface::Workspace));
                            if editing {
                                if let Some(sid) = &face.sid {
                                    self.inline_rename(
                                        ui,
                                        sid,
                                        RenameSurface::Workspace,
                                        egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x + label_x, rect.min.y + 8.0),
                                            egui::pos2(
                                                rect.max.x - (28.0 + badge_reserve),
                                                rect.max.y - 7.0,
                                            ),
                                        ),
                                    );
                                }
                            } else {
                                let mut text = egui::text::LayoutJob::simple(
                                    face.label.clone(),
                                    egui::FontId::proportional(13.0),
                                    tint,
                                    rect.width() - 58.0 - (label_x - 30.0) - badge_reserve,
                                );
                                text.wrap.max_rows = 1;
                                text.wrap.break_anywhere = true;
                                let galley = ui.painter().layout_job(text);
                                ui.painter().galley(
                                    egui::pos2(
                                        rect.left() + label_x,
                                        rect.center().y - galley.size().y * 0.5,
                                    ),
                                    galley,
                                    tint,
                                );
                            }
                            // Aggregate attention across the tab's terminals.
                            if let Some((count, _)) = &badge {
                                let badge_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.right() - 44.0, rect.center().y - 8.0),
                                    egui::pos2(rect.right() - 26.0, rect.center().y + 8.0),
                                );
                                ui.painter().text(
                                    badge_rect.right_center(),
                                    egui::Align2::RIGHT_CENTER,
                                    count,
                                    egui::FontId::proportional(11.0),
                                    appearance::color(if attention.waiting() > 0 {
                                        &self.theme.status_waiting
                                    } else {
                                        &self.theme.status_failed
                                    }),
                                );
                                #[cfg(feature = "test-support")]
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab-attention:{}", face.label),
                                    badge_rect,
                                );
                            }
                            if active {
                                // Inset from the bottom edge. A stroke centered
                                // on rect.bottom() is clipped away by the strip.
                                let underline = egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0),
                                    egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
                                );
                                ui.painter().rect_filled(
                                    underline,
                                    1.0,
                                    appearance::color(&self.theme.accent),
                                );
                                #[cfg(feature = "test-support")]
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab-underline:{}", face.label),
                                    underline,
                                );
                            }
                            let close_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                egui::vec2(20.0, 24.0),
                            );
                            let close_response = ui
                                .interact(
                                    close_rect,
                                    egui::Id::new(("close-workspace", project, &group.id)),
                                    egui::Sense::click(),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Close tab");
                            if response.hovered() || close_response.hovered() || active {
                                egui::Image::new(icons::source("X"))
                                    .tint(appearance::ICON_COLOR)
                                    .paint_at(
                                        ui,
                                        egui::Rect::from_center_size(
                                            close_rect.center(),
                                            egui::vec2(16.0, 16.0),
                                        ),
                                    );
                            }
                            if close_response.clicked() {
                                close = Some(group.id.clone());
                            }
                            if response.clicked()
                                && !editing
                                && self.pane_drag.is_none()
                                && self.tab_drag.is_none()
                                && !close_rect.contains(
                                    response.interact_pointer_pos().unwrap_or(egui::Pos2::ZERO),
                                )
                            {
                                switch = Some(group.id.clone());
                            }
                            if response.double_clicked()
                                && !editing
                                && self.tab_drag.is_none()
                                && let Some(sid) = &face.sid
                            {
                                self.begin_rename(sid, RenameSurface::Workspace);
                            }
                            response
                                .clone()
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_ui(|ui| {
                                    let mut tooltip = tab_tooltip(
                                        &face.label,
                                        face.sid.as_ref().and_then(|sid| {
                                            self.state
                                                .sessions
                                                .iter()
                                                .find(|s| &s.id == sid)
                                                .map(|s| s.cwd.as_path())
                                        }),
                                    );
                                    if let Some(sid) = &face.sid
                                        && let Some(first) = self
                                            .present_session(sid)
                                            .diagnostics(now())
                                            .lines()
                                            .next()
                                    {
                                        tooltip.push_str(&format!("\n{first}"));
                                    }
                                    if let Some((_, breakdown)) = &badge {
                                        tooltip.push_str(&format!("\nAttention: {breakdown}"));
                                    }
                                    ui.label(tooltip);
                                });
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    true,
                                    active,
                                    &face.label,
                                )
                            });
                            appearance::context_menu(&response, |ui| {
                                let action = self.workspace_tab_menu(
                                    ui,
                                    WorkspaceTabMenuSpec {
                                        sid: face.sid.as_deref(),
                                        index,
                                        count: tab_count,
                                    },
                                );
                                if action.close {
                                    close = Some(group.id.clone());
                                }
                                if action.close_all {
                                    close_all = true;
                                }
                                if action.close_left {
                                    close_left = Some(group.id.clone());
                                }
                                if action.close_right {
                                    close_right = Some(group.id.clone());
                                }
                                if action.add_left {
                                    add_at = Some(index);
                                }
                                if action.add_right {
                                    add_at = Some(index.saturating_add(1));
                                }
                            });
                            #[cfg(feature = "test-support")]
                            {
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab:{}", face.label),
                                    rect,
                                );
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-close:{}", face.label),
                                    close_rect,
                                );
                            }
                        }
                    });
                });
            if overflow {
                let max_offset = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
                let right = ui
                    .add_enabled(
                        scroll.state.offset.x < max_offset - 0.5,
                        egui::Button::new("›").min_size(egui::vec2(27.0, 30.0)),
                    )
                    .on_hover_text("Scroll tabs right");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "tabs-right", right.rect);
                if right.clicked() {
                    direction = 1.0;
                }
                if direction != 0.0 {
                    scroll.state.offset.x = (scroll.state.offset.x
                        + direction * scroll.inner_rect.width().max(1.0) * 0.8)
                        .clamp(0.0, max_offset);
                    scroll.state.store(ui.ctx(), scroll.id);
                    ui.ctx().request_repaint();
                }
            }
            let plus_response = ui
                .add_sized(
                    [30.0, 30.0],
                    egui::Button::new(RichText::new("+").size(18.0)).frame(false),
                )
                .on_hover_text("New top-level terminal tab");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "workspace-plus", plus_response.rect);
            let plus_rect = plus_response.rect;
            if plus_response.clicked() && self.pane_drag.is_none() {
                self.create(None);
            }
            // Dropping a dragged terminal pane onto a strip tab moves it
            // into that tab (its focused split; the tab content was already
            // previewed on hover); dropping into a gap between tabs, onto
            // "+", or onto empty strip background opens it in a fresh
            // top-level tab at that slot. Dropping a dragged strip tab
            // reorders it to the insertion slot instead.
            let released = ui.input(|i| i.pointer.any_released());
            if let Some(pane) = self.pane_drag.clone()
                && drag_same_project
                && released
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                if let Some(group_id) = Self::strip_interior_tab(&strip_rects, pos) {
                    if self.pane_drag_from_strip
                        && let Tab::Terminal(sid) = &pane
                    {
                        self.pull_strip_shell_into(workspace, sid);
                    }
                    if workspace.move_pane_to_group(&pane, &group_id) {
                        if let Tab::Terminal(sid) = &pane {
                            self.active_session = Some(sid.clone());
                            self.focus_tab = Some(pane.clone());
                        }
                        self.pane_index = None;
                        switch = None;
                    }
                    self.drop_preview_origin = None;
                    self.end_pane_drag();
                } else {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index {
                        if self.pane_drag_from_strip
                            && let Tab::Terminal(sid) = &pane
                        {
                            self.pull_strip_shell_into(workspace, sid);
                        }
                        if workspace.move_pane_to_new_group_at(&pane, index).is_some() {
                            if let Tab::Terminal(sid) = &pane {
                                self.active_session = Some(sid.clone());
                                self.focus_tab = Some(pane.clone());
                            }
                            self.pane_index = None;
                            switch = None;
                        }
                        self.drop_preview_origin = None;
                        self.end_pane_drag();
                    }
                }
            }
            // A released tab drag the strip did not consume was dropped
            // outside of it: cancel without moving anything.
            if self.tab_drag.is_some() && released {
                if let (Some(dragged), Some(pos)) = (
                    self.tab_drag.clone(),
                    ui.input(|i| i.pointer.interact_pos()),
                ) && workspace.tabs.iter().any(|tab| tab.id == dragged)
                {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index
                        && workspace.reorder_group(&dragged, index)
                    {
                        workspace.active = dragged;
                        switch = None;
                    }
                }
                self.tab_drag = None;
            }
            if self.tab_drag.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.tab_drag = None;
            }
            // The dock paints the previewed tab's focused leaf at real size
            // while a pane hovers a strip tab interior; hovering a gap,
            // "+", or empty strip background shows the tab ghost instead
            // of the pane snapshot ghost. Both mirror the drop routing.
            let strip_pos = (!released && ui.input(|i| i.pointer.any_down()))
                .then(|| ui.input(|i| i.pointer.interact_pos()))
                .flatten();
            self.strip_tab_hover = drag_same_project
                && strip_pos
                    .is_some_and(|pos| Self::strip_interior_tab(&strip_rects, pos).is_some());
            self.strip_new_tab_hover = drag_same_project
                && strip_pos.is_some_and(|pos| {
                    Self::strip_interior_tab(&strip_rects, pos).is_none()
                        && (plus_rect.contains(pos)
                            || Self::strip_insertion_at(&strip_rect, &strip_rects, pos).is_some())
                });
            // Insertion preview: hovering a strip gap, "+", or empty strip
            // background while dragging a pane paints the slot where the
            // fresh top-level tab will land; dragging a strip tab paints
            // the slot it will reorder into. Mirrors the pane drop wash.
            if !released
                && ui.input(|i| i.pointer.any_down())
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                let accent = appearance::color(&self.theme.accent);
                let pane_gap = drag_same_project
                    && self.pane_drag.is_some()
                    && Self::strip_interior_tab(&strip_rects, pos).is_none();
                let tab_member = self
                    .tab_drag
                    .clone()
                    .is_some_and(|dragged| workspace.tabs.iter().any(|tab| tab.id == dragged));
                if pane_gap || tab_member {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index {
                        if plus_rect.contains(pos) {
                            ui.painter().rect_stroke(
                                plus_rect,
                                4,
                                egui::Stroke::new(2.0, accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        Self::paint_strip_insertion(ui, &strip_rect, &strip_rects, index, accent);
                        if let Some(dragged) = self.tab_drag.clone()
                            && let Some((_, rect)) =
                                strip_rects.iter().find(|(id, _)| id == &dragged)
                        {
                            ui.painter().rect_stroke(
                                *rect,
                                0,
                                egui::Stroke::new(2.0, accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                        ui.ctx().request_repaint();
                    }
                }
            }
            if self.pane_drag.is_some() {
                ui.ctx().request_repaint();
            }
            header_drag_space(ui);
        });
        ui.painter().hline(
            strip_rect.x_range(),
            strip_rect.bottom(),
            egui::Stroke::new(1.0, appearance::color(&self.theme.border)),
        );
        ui.add_space(1.0);
        ui.spacing_mut().item_spacing = previous_spacing;
        if let Some(id) = switch {
            if workspace.active != id && self.rename_surface == RenameSurface::Pane {
                self.finish_rename(true);
            }
            workspace.active = id;
            self.hover_popup = None;
            self.hide_center_overlay();
        }
        if let Some(id) = close {
            self.begin_workspace_close_tabs(project, vec![id]);
        }
        if close_all {
            self.begin_workspace_close_tabs(project, workspace.ids());
        }
        if let Some(id) = close_left {
            self.begin_workspace_close_tabs(project, workspace.ids_before(&id));
        }
        if let Some(id) = close_right {
            self.begin_workspace_close_tabs(project, workspace.ids_after(&id));
        }
        if let Some(index) = add_at {
            self.create_workspace_tab(Some(index));
        }
    }
}
