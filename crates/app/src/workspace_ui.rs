//! Workspace and terminal rendering.
use super::*;

impl App {
    pub(super) fn terminal_input_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.settings_open
            && !self.command_dialog_open()
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && self.rename_session.is_none()
            && !self.open_path
            && self.search_session.is_none()
    }

    pub(super) fn window_header(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let left = self.project_width.min(rect.width() - 300.0);
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
                .max_rect(left_rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if cfg!(target_os = "macos") {
                    let native = ui
                        .input(|i| i.viewport().native_pixels_per_point)
                        .unwrap_or(ui.ctx().pixels_per_point());
                    ui.add_space(72.0 * native / ui.ctx().pixels_per_point());
                } else {
                    for (label, command) in [
                        ("×", egui::ViewportCommand::Close),
                        ("−", egui::ViewportCommand::Minimized(true)),
                        (
                            "□",
                            egui::ViewportCommand::Maximized(
                                !ui.input(|i| i.viewport().maximized.unwrap_or(false)),
                            ),
                        ),
                    ] {
                        if ui.small_button(label).clicked() {
                            ui.ctx().send_viewport_cmd(command);
                        }
                    }
                }
                let response = ui.add(
                    egui::Label::new(
                        RichText::new(
                            self.selected_project()
                                .map_or("Terminator", |p| p.name.as_str()),
                        )
                        .strong(),
                    )
                    .truncate()
                    .sense(egui::Sense::drag()),
                );
                if response.drag_started() {
                    begin_native_window_gesture(ui.ctx(), egui::ViewportCommand::StartDrag);
                }
                header_drag_space(ui);
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
            ui.set_clip_rect(tabs_rect);
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
        });
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(tools_rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tool, label) in [
                    (SidebarTool::Explorer, "Explorer"),
                    (SidebarTool::Agents, "Agents"),
                    (SidebarTool::Git, "Git"),
                    (SidebarTool::History, "History"),
                ] {
                    let response = appearance::tool_button(
                        ui,
                        tool,
                        label,
                        self.preferences.visible && self.preferences.tool == tool,
                    );
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("tool-{label}"), response.rect);
                    let response = if tool == SidebarTool::Explorer {
                        response.on_hover_text(self.explorer_tooltip())
                    } else {
                        response
                    };
                    if response.clicked() {
                        self.preferences.toggle(tool);
                    }
                }
                let settings_tip = {
                    let keys = shortcuts::pretty(&self.state.settings.keybindings, "open_settings");
                    if keys.is_empty() {
                        "Settings".into()
                    } else {
                        format!("Settings ({keys})")
                    }
                };
                let settings = ui
                    .add_sized(
                        [36.0, 32.0],
                        egui::Button::image(
                            egui::Image::new(icons::source("Settings"))
                                .tint(appearance::ICON_COLOR)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                        )
                        .frame(false),
                    )
                    .on_hover_text(settings_tip);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "settings", settings.rect);
                if settings.clicked() {
                    self.open_settings();
                }
                let palette = ui
                    .add_sized(
                        [36.0, 32.0],
                        egui::Button::image(
                            egui::Image::new(icons::source("Search"))
                                .tint(appearance::ICON_COLOR)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                        )
                        .frame(false),
                    )
                    .on_hover_text({
                        let keys =
                            shortcuts::pretty(&self.state.settings.keybindings, "open_palette");
                        if keys.is_empty() {
                            "Command palette".into()
                        } else {
                            format!("Command palette ({keys})")
                        }
                    });
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "palette", palette.rect);
                if palette.clicked() {
                    self.palette_open = true;
                    self.palette_query.clear();
                    self.palette_index = 0;
                }
                header_drag_space(ui);
            },
        );
    }
    pub(super) fn workspace_bar(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        workspace: &mut Workspace,
    ) {
        let mut switch = None;
        let mut close = None;
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
            let width = (ui.available_width() - 38.0).max(40.0);
            let overflow = workspace.tabs.len() as f32 * 221.0 > width;
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
            let mut scroll = egui::ScrollArea::horizontal()
                .id_salt(("workspace-tabs", project))
                .max_width(width)
                .auto_shrink([true, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.horizontal(|ui| {
                        for group in &workspace.tabs {
                            let primary = group
                                .primary
                                .as_ref()
                                .filter(|tab| group.layout.find_tab(tab).is_some())
                                .or_else(|| {
                                    group.layout.iter_all_tabs().next().map(|(_, tab)| tab)
                                });
                            let (label, icon, sid) = match primary {
                                Some(Tab::Terminal(sid)) => self
                                    .state
                                    .sessions
                                    .iter()
                                    .find(|s| &s.id == sid)
                                    .map(|s| {
                                        (
                                            s.label.clone(),
                                            if s.kind == SessionKind::Editor {
                                                "FileCode"
                                            } else {
                                                "Terminal"
                                            },
                                            Some(sid.clone()),
                                        )
                                    })
                                    .unwrap_or(("Terminal".into(), "Terminal", None)),
                                Some(Tab::Diff { path, .. })
                                | Some(Tab::Image { path })
                                | Some(Tab::Html { path }) => (
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned(),
                                    if matches!(primary, Some(Tab::Image { .. })) {
                                        "FileImage"
                                    } else if matches!(primary, Some(Tab::Html { .. })) {
                                        "FileCode"
                                    } else {
                                        "FileDiff"
                                    },
                                    None,
                                ),
                                Some(Tab::Player) => ("Player".into(), "FileMusic", None),
                                None => ("Workspace".into(), "Terminal", None),
                            };
                            let active = workspace.active == group.id;
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(220.0, 32.0), egui::Sense::click());
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
                            let icon_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 16.0, rect.center().y),
                                egui::vec2(16.0, 16.0),
                            );
                            if icon == "Terminal" {
                                ui.painter().rect_filled(icon_rect, 2, egui::Color32::BLACK);
                            }
                            egui::Image::new(icons::source(icon))
                                .tint(appearance::ICON_COLOR)
                                .paint_at(
                                    ui,
                                    if icon == "Terminal" {
                                        icon_rect.shrink(1.0)
                                    } else {
                                        icon_rect
                                    },
                                );
                            let editing = sid
                                .as_ref()
                                .is_some_and(|sid| self.renaming(sid, RenameSurface::Workspace));
                            if editing {
                                if let Some(sid) = &sid {
                                    self.inline_rename(
                                        ui,
                                        sid,
                                        RenameSurface::Workspace,
                                        egui::Rect::from_min_max(
                                            rect.min + egui::vec2(30.0, 8.0),
                                            rect.max - egui::vec2(28.0, 7.0),
                                        ),
                                    );
                                }
                            } else {
                                let mut text = egui::text::LayoutJob::simple(
                                    label.clone(),
                                    egui::FontId::proportional(13.0),
                                    tint,
                                    rect.width() - 58.0,
                                );
                                text.wrap.max_rows = 1;
                                text.wrap.break_anywhere = true;
                                let galley = ui.painter().layout_job(text);
                                ui.painter().galley(
                                    egui::pos2(
                                        rect.left() + 30.0,
                                        rect.center().y - galley.size().y * 0.5,
                                    ),
                                    galley,
                                    tint,
                                );
                            }
                            if active {
                                ui.painter().line_segment(
                                    [rect.left_bottom(), rect.right_bottom()],
                                    egui::Stroke::new(
                                        2.0,
                                        appearance::color(&self.theme.secondary),
                                    ),
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
                                && !close_rect.contains(
                                    response.interact_pointer_pos().unwrap_or(egui::Pos2::ZERO),
                                )
                            {
                                switch = Some(group.id.clone());
                            }
                            if response.double_clicked()
                                && !editing
                                && let Some(sid) = &sid
                            {
                                self.begin_rename(sid, RenameSurface::Workspace);
                            }
                            response
                                .clone()
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(&label);
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    true,
                                    active,
                                    &label,
                                )
                            });
                            appearance::context_menu(&response, |ui| {
                                if let Some(sid) = &sid {
                                    self.rename_action(ui, sid, RenameSurface::Workspace);
                                }
                                if appearance::menu_item(ui, "Close tab…", "X", "").clicked() {
                                    close = Some(group.id.clone());
                                    ui.close();
                                }
                            });
                            #[cfg(feature = "test-support")]
                            {
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab:{label}"),
                                    rect,
                                );
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-close:{label}"),
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
            let response = ui
                .add_sized(
                    [30.0, 30.0],
                    egui::Button::new(RichText::new("+").size(18.0)).frame(false),
                )
                .on_hover_text("New top-level terminal tab");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "workspace-plus", response.rect);
            if response.clicked() {
                self.create(None);
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
        }
        if let Some(id) = close {
            self.rename_session = None;
            self.close_workspace = Some((project.into(), id));
        }
    }
    fn new_terminal_menu(&mut self, ui: &mut egui::Ui, pane: Option<egui_dock::NodePath>) {
        for (label, split, action) in [
            ("New tab", None, "new_terminal"),
            ("Split up", Some("up"), ""),
            ("Split down", Some("down"), "split_down"),
            ("Split left", Some("left"), ""),
            ("Split right", Some("right"), "split_right"),
        ] {
            let icon = match split {
                Some("up") => "PanelTopClose",
                Some("down") => "PanelBottomClose",
                Some("left") => "PanelLeftClose",
                Some("right") => "PanelRightClose",
                _ => "Plus",
            };
            let shortcut = shortcuts::pretty(&self.state.settings.keybindings, action);
            if appearance::menu_item(ui, label, icon, &shortcut).clicked() {
                if let Some(pane) = pane {
                    self.add_tab = Some((pane, split.map(str::to_owned)));
                } else {
                    self.create(split);
                }
                ui.close();
            }
        }
        if let Some(tabs) = pane
            .and_then(|pane| self.pane_tabs.get(&pane))
            .cloned()
            .filter(|tabs| tabs.len() > 1)
        {
            ui.separator();
            ui.weak("Tabs in this pane");
            for tab in tabs {
                let label = match &tab {
                    Tab::Terminal(sid) => self
                        .state
                        .sessions
                        .iter()
                        .find(|s| &s.id == sid)
                        .map(|s| s.label.clone())
                        .unwrap_or_else(|| "Terminal".into()),
                    Tab::Diff { path, .. } | Tab::Image { path } | Tab::Html { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    Tab::Player => "Player".into(),
                };
                if appearance::menu_item(ui, &label, "Terminal", "").clicked() {
                    self.focus_tab = Some(tab);
                    ui.close();
                }
            }
        }
    }
    pub(super) fn rename_action(&mut self, ui: &mut egui::Ui, sid: &str, surface: RenameSurface) {
        if appearance::menu_item(ui, "Rename terminal…", "Pencil", "").clicked() {
            self.begin_rename(sid, surface);
            ui.close();
        }
    }
    pub(super) fn inline_rename(
        &mut self,
        ui: &mut egui::Ui,
        sid: &str,
        surface: RenameSurface,
        rect: egui::Rect,
    ) {
        if !self.renaming(sid, surface) {
            return;
        }
        let mut title = self.rename_session.as_ref().unwrap().1.clone();
        let starting = self.rename_focus;
        let response = ui.put(
            rect,
            egui::TextEdit::singleline(&mut title)
                .id_salt(("inline-terminal-title", sid, surface))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(rect.width()),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "rename-input", response.rect);
        if starting {
            response.request_focus();
            self.rename_focus = false;
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(title.chars().count()),
                    )));
                state.store(ui.ctx(), response.id);
            }
        }
        let valid = !title.trim().is_empty() && title.trim().len() <= 256;
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        // The field owns this frame's keyboard input, including when Enter or
        // blur commits it before a terminal widget is rendered later in the frame.
        ui.input_mut(|input| {
            input.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Key { .. }
                )
            })
        });
        if escape || (!starting && response.lost_focus() && !valid) {
            self.rename_session = None;
            response.surrender_focus();
        } else if valid && (enter || (!starting && response.lost_focus())) {
            self.send(Request::Rename {
                session: sid.into(),
                label: title.trim().into(),
            });
            self.rename_session = None;
            response.surrender_focus();
        } else {
            self.rename_session = Some((sid.into(), title));
            if enter {
                response.request_focus();
            }
            response.on_hover_text(if valid {
                "Enter to save · Esc to cancel"
            } else {
                "Enter a non-empty, shorter title"
            });
        }
    }
    fn image_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_images.insert(path.into());
        let mut as_text = false;
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(path.display().to_string()).truncate())
                .on_hover_text(path.display().to_string());
            if ui.button("Reload").clicked() {
                self.images.remove(path);
            }
            as_text = ui.button("Open as text").clicked();
            if ui.button("Open externally").clicked() {
                let _ = self.jobs.send(Job::External(path.into()));
            }
        });
        if as_text {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if !self.images.contains_key(path) && self.images.len() >= 8 {
            ui.weak("Close another image preview to load this image.");
            return;
        }
        let preview = self.images.entry(path.into()).or_default();
        if !preview.loading && preview.texture.is_none() && preview.error.is_none() {
            self.image_generation = self.image_generation.wrapping_add(1);
            preview.generation = self.image_generation;
            preview.loading = self
                .image_jobs
                .try_send((path.into(), preview.generation))
                .is_ok();
        }
        preview.show(ui);
    }
    fn html_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_htmls.insert(path.into());
        let mut as_text = false;
        let mut browser = false;
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(path.display().to_string()).truncate())
                .on_hover_text(path.display().to_string());
            if ui.button("Reload").clicked() {
                self.htmls.remove(path);
            }
            as_text = ui.button("Open as text").clicked();
            let open = ui.button("Open in browser");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "html-open-browser", open.rect);
            browser = open.clicked();
        });
        if as_text {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if browser {
            self.open_in_browser(path);
        }
        if !self.htmls.contains_key(path) && self.htmls.len() >= 4 {
            ui.weak("Close another HTML preview to load this page.");
            return;
        }
        let preview = self.htmls.entry(path.into()).or_default();
        if !preview.loading && preview.texture.is_none() && preview.error.is_none() {
            self.html_generation = self.html_generation.wrapping_add(1);
            preview.generation = self.html_generation;
            preview.loading = self
                .html_jobs
                .try_send((path.into(), preview.generation))
                .is_ok();
        }
        preview.show(ui);
        if preview.error.is_some() {
            ui.weak("Open in browser to view the page in your system browser.");
        }
    }
    fn diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Diff { path, staged, .. } = tab else {
            return;
        };
        let key = tab.key();
        ui.horizontal(|ui| {
            ui.weak(path.display().to_string());
            ui.weak(if *staged {
                "HEAD → Index"
            } else {
                "Index → Working tree"
            });
            let split = self.diff_split.contains(&key);
            if ui.selectable_label(!split, "Unified").clicked() {
                self.diff_split.remove(&key);
            }
            let side_by_side = ui.selectable_label(split, "Side by side");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-side-by-side", side_by_side.rect);
            if side_by_side.clicked() {
                self.diff_split.insert(key.clone());
            }
            if ui.small_button("Refresh").clicked() {
                self.diffs.remove(&key);
                self.loading.insert(key.clone());
                let _ = self.jobs.send(Job::Diff(tab.clone()));
            }
        });
        if !self.diffs.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::Diff(tab.clone()));
        }
        match self.diffs.get(&key) {
            Some(Ok(doc)) => {
                let split = self.diff_split.contains(&key);
                let colors = DiffColors {
                    added: appearance::color(&self.theme.git_added),
                    deleted: appearance::color(&self.theme.git_deleted),
                    accent: appearance::color(&self.theme.accent),
                    text: appearance::color(&self.theme.text),
                };
                let doc = doc.clone();
                paint_diff_document(ui, &doc, split, colors, &key);
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
            None => {
                ui.spinner();
            }
        }
    }
}

#[derive(Clone, Copy)]
struct DiffColors {
    added: Color32,
    deleted: Color32,
    accent: Color32,
    text: Color32,
}

#[derive(Clone, Copy)]
enum DiffGutter {
    Unified,
    Old,
    New,
}

struct DiffMetrics {
    font: egui::FontId,
    digit_w: f32,
    row_h: f32,
    digits: u32,
}

impl DiffMetrics {
    fn measure(ui: &egui::Ui, digits: u32) -> Self {
        let font = ui.style().text_styles[&egui::TextStyle::Monospace].clone();
        let (digit_w, row_h) = ui
            .ctx()
            .fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
        Self {
            font,
            digit_w,
            row_h,
            digits,
        }
    }

    fn number_w(&self) -> f32 {
        self.digit_w * self.digits as f32
    }
}

#[derive(Clone, Copy)]
struct GutterCols {
    old_right: Option<f32>,
    new_right: Option<f32>,
    sign: f32,
    code: f32,
}

struct DiffPaint<'a> {
    width: f32,
    line: &'a diff::DiffLine,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffSidePaint<'a> {
    size: egui::Vec2,
    line: Option<&'a diff::DiffLine>,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffGutterPaint<'a> {
    rect: egui::Rect,
    cols: GutterCols,
    line: &'a diff::DiffLine,
    sign: &'static str,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffNumberPaint<'a> {
    top: f32,
    right: f32,
    number: Option<u32>,
    metrics: &'a DiffMetrics,
    color: Color32,
}

// Widths are document-wide so vertical virtualization cannot shrink the horizontal
// scroll range when the longest line leaves the viewport.
#[derive(Clone)]
struct DiffWidths {
    fingerprint: egui::Id,
    content: f32,
}

fn diff_content_width(
    ui: &egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    metrics: &DiffMetrics,
    scroll_key: &str,
) -> f32 {
    let cache_id = egui::Id::new(("diff-width", scroll_key, split));
    let fingerprint = egui::Id::new((
        doc,
        &metrics.font,
        metrics.digit_w.to_bits(),
        metrics.row_h.to_bits(),
        ui.ctx().pixels_per_point().to_bits(),
    ));
    if let Some(cached) = ui
        .ctx()
        .data_mut(|data| data.get_temp::<DiffWidths>(cache_id))
        && cached.fingerprint == fingerprint
    {
        return cached.content;
    }
    let lines: Box<dyn Iterator<Item = &diff::DiffLine>> = if split {
        Box::new(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        Box::new(doc.unified.iter())
    };
    let gutter = if split {
        DiffGutter::Old
    } else {
        DiffGutter::Unified
    };
    let code_width = lines
        .map(|line| {
            let text = line
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>();
            ui.painter()
                .layout_no_wrap(text, metrics.font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let content = gutter_cols(metrics, gutter).code + code_width;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            cache_id,
            DiffWidths {
                fingerprint,
                content,
            },
        )
    });
    content
}

fn paint_diff_document(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    colors: DiffColors,
    scroll_key: &str,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let rows = diff_row_count(doc, split);
    let digits = diff_doc_digits(doc, split);
    let metrics = DiffMetrics::measure(ui, digits);
    let content = diff_content_width(ui, doc, split, &metrics, scroll_key);
    let columns = if split { 2.0 } else { 1.0 };
    let column_width = content.max(ui.available_width() / columns);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        egui::ScrollArea::both()
            .id_salt((scroll_key, split))
            .auto_shrink([false, false])
            .show_rows(ui, metrics.row_h, rows, |ui, range| {
                ui.set_min_width(column_width * columns);
                for index in range {
                    if split {
                        paint_diff_split_row(ui, &doc.split[index], colors, &metrics, column_width);
                    } else {
                        paint_diff_line(
                            ui,
                            DiffPaint {
                                width: column_width,
                                line: &doc.unified[index],
                                gutter: DiffGutter::Unified,
                                colors,
                                metrics: &metrics,
                            },
                        );
                    }
                }
            })
    })
    .inner
}

fn paint_diff_split_row(
    ui: &mut egui::Ui,
    row: &diff::SplitRow,
    colors: DiffColors,
    metrics: &DiffMetrics,
    width: f32,
) {
    let size = egui::vec2(width, metrics.row_h);
    ui.allocate_ui_with_layout(
        egui::vec2(width * 2.0, metrics.row_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            paint_diff_side(
                ui,
                DiffSidePaint {
                    size,
                    line: row.left.as_ref(),
                    gutter: DiffGutter::Old,
                    colors,
                    metrics,
                },
            );
            paint_diff_side(
                ui,
                DiffSidePaint {
                    size,
                    line: row.right.as_ref(),
                    gutter: DiffGutter::New,
                    colors,
                    metrics,
                },
            );
        },
    );
}

fn diff_row_count(doc: &diff::DiffDocument, split: bool) -> usize {
    if split {
        doc.split.len()
    } else {
        doc.unified.len()
    }
}

fn diff_doc_digits(doc: &diff::DiffDocument, split: bool) -> u32 {
    if split {
        diff_gutter_digits(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        diff_gutter_digits(doc.unified.iter())
    }
}

fn diff_gutter_digits<'a>(lines: impl IntoIterator<Item = &'a diff::DiffLine>) -> u32 {
    let max = lines
        .into_iter()
        .flat_map(|line| [line.old_no, line.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    max.ilog10().saturating_add(1).max(4)
}

fn gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let number_w = metrics.number_w();
    let mut x = pad;
    match gutter {
        DiffGutter::Unified => {
            let old_right = x + number_w;
            x = old_right + pad;
            let new_right = x + number_w;
            x = new_right + pad;
            let sign = x;
            GutterCols {
                old_right: Some(old_right),
                new_right: Some(new_right),
                sign,
                code: sign + metrics.digit_w + pad,
            }
        }
        DiffGutter::Old | DiffGutter::New => side_gutter_cols(metrics, gutter),
    }
}

fn side_gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let right = pad + metrics.number_w();
    let sign = right + pad;
    let code = sign + metrics.digit_w + pad;
    GutterCols {
        old_right: matches!(gutter, DiffGutter::Old).then_some(right),
        new_right: matches!(gutter, DiffGutter::New).then_some(right),
        sign,
        code,
    }
}

fn paint_diff_side(ui: &mut egui::Ui, paint: DiffSidePaint<'_>) {
    let DiffSidePaint {
        size,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    ui.allocate_ui(size, |ui| {
        ui.set_min_size(size);
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
        if let Some(line) = line {
            paint_diff_line(
                ui,
                DiffPaint {
                    width: size.x,
                    line,
                    gutter,
                    colors,
                    metrics,
                },
            );
        }
    });
}

fn diff_row_style(kind: diff::LineKind, colors: DiffColors) -> (Color32, &'static str) {
    match kind {
        diff::LineKind::Insert => (tint(colors.added, 40), "+"),
        diff::LineKind::Delete => (tint(colors.deleted, 40), "-"),
        diff::LineKind::Hunk => (tint(colors.accent, 24), " "),
        diff::LineKind::Equal => (Color32::TRANSPARENT, " "),
    }
}

fn diff_code_job(
    line: &diff::DiffLine,
    font: &egui::FontId,
    colors: DiffColors,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping::no_max_width(),
        break_on_newline: false,
        ..Default::default()
    };
    for span in &line.spans {
        let intra = match (line.kind, span.intra) {
            (diff::LineKind::Insert, diff::Intra::Change) => tint(colors.added, 90),
            (diff::LineKind::Delete, diff::Intra::Change) => tint(colors.deleted, 90),
            _ => Color32::TRANSPARENT,
        };
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: if line.kind == diff::LineKind::Hunk {
                    colors.accent
                } else {
                    Color32::from_rgb(span.rgb[0], span.rgb[1], span.rgb[2])
                },
                background: intra,
                ..Default::default()
            },
        );
    }
    if job.text.is_empty() {
        job.append(
            " ",
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: colors.text,
                ..Default::default()
            },
        );
    }
    job
}

fn fill_diff_row(ui: &egui::Ui, rect: egui::Rect, gutter_w: f32, bg: Color32) {
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let gutter =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
    ui.painter()
        .rect_filled(gutter, 0.0, Color32::from_black_alpha(40));
}

fn paint_diff_number(ui: &egui::Ui, paint: DiffNumberPaint<'_>) {
    let DiffNumberPaint {
        top,
        right,
        number,
        metrics,
        color,
    } = paint;
    let Some(number) = number else {
        return;
    };
    let galley = ui
        .painter()
        .layout_no_wrap(number.to_string(), metrics.font.clone(), color);
    ui.painter().galley(
        egui::pos2(right - galley.size().x, top).round(),
        galley,
        color,
    );
}

fn paint_diff_gutter(ui: &egui::Ui, paint: DiffGutterPaint<'_>) {
    let DiffGutterPaint {
        rect,
        cols,
        line,
        sign,
        colors,
        metrics,
    } = paint;
    if line.kind == diff::LineKind::Hunk {
        return;
    }
    let weak = ui.visuals().weak_text_color();
    if let Some(right) = cols.old_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.old_no,
                metrics,
                color: weak,
            },
        );
    }
    if let Some(right) = cols.new_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.new_no,
                metrics,
                color: weak,
            },
        );
    }
    if sign != " " {
        let color = match line.kind {
            diff::LineKind::Insert => colors.added,
            diff::LineKind::Delete => colors.deleted,
            diff::LineKind::Hunk | diff::LineKind::Equal => weak,
        };
        let galley = ui
            .painter()
            .layout_no_wrap(sign.to_owned(), metrics.font.clone(), color);
        ui.painter().galley(
            egui::pos2(rect.left() + cols.sign, rect.top()).round(),
            galley,
            color,
        );
    }
}

fn paint_diff_line(ui: &mut egui::Ui, paint: DiffPaint<'_>) {
    let DiffPaint {
        width,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    let (bg, sign) = diff_row_style(line.kind, colors);
    let cols = gutter_cols(metrics, gutter);
    let code = ui
        .painter()
        .layout_job(diff_code_job(line, &metrics.font, colors));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, metrics.row_h), egui::Sense::hover());
    fill_diff_row(ui, rect, cols.code, bg);
    paint_diff_gutter(
        ui,
        DiffGutterPaint {
            rect,
            cols,
            line,
            sign,
            colors,
            metrics,
        },
    );
    ui.painter().galley(
        (rect.left_top() + egui::vec2(cols.code, 0.0)).round(),
        code,
        colors.text,
    );
}

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub(super) struct Viewer<'a> {
    pub(super) app: &'a mut App,
}
impl TabViewer for Viewer<'_> {
    fn on_add(&mut self, path: egui_dock::NodePath) {
        self.app.add_tab = Some((path, None));
    }
    type Tab = Tab;
    fn show_tab_bar(&self, _path: egui_dock::NodePath) -> bool {
        false
    }
    fn trailing_controls_width(&self) -> f32 {
        28.0
    }
    fn trailing_controls(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        #[cfg(feature = "test-support")]
        {
            let rect = ui.max_rect();

            diagnostics::record(
                ui.ctx(),
                "pane-plus",
                rect.translate(egui::vec2(-24.0, 0.0)),
            );
        }
        let response = appearance::menu_button(ui, "⌄", |ui| {
            for (label, direction) in [
                ("New tab", None),
                ("Split up", Some("up")),
                ("Split down", Some("down")),
                ("Split left", Some("left")),
                ("Split right", Some("right")),
            ] {
                let response = appearance::menu_item(
                    ui,
                    label,
                    match direction {
                        Some("up") => "PanelTopClose",
                        Some("down") => "PanelBottomClose",
                        Some("left") => "PanelLeftClose",
                        Some("right") => "PanelRightClose",
                        _ => "Plus",
                    },
                    "",
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), label, response.rect);
                if response.clicked() {
                    self.app.add_tab = Some((path, direction.map(str::to_owned)));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("New tab or split this pane");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-dropdown", response.rect);
        let _ = response;
    }
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(tab.key())
    }
    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Image { path } | Tab::Html { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
            Tab::Player => "Player".into(),
            Tab::Terminal(sid) => self
                .app
                .state
                .sessions
                .iter()
                .find(|s| s.id == *sid)
                .map(|s| s.label.clone())
                .unwrap_or("Session".into())
                .into(),
            Tab::Diff { path, staged, .. } => format!(
                "{} {}",
                if *staged { "Staged:" } else { "Diff:" },
                path.file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
        }
    }
    fn allowed_in_windows(&self, _: &mut Tab) -> bool {
        false
    }
    fn scroll_bars(&self, _: &Tab) -> [bool; 2] {
        [false, false]
    }
    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        if matches!(tab, Tab::Player) {
            self.app.player.stop();
        }
        if let Tab::Terminal(sid) = tab {
            if self
                .app
                .state
                .sessions
                .iter()
                .any(|s| s.id == *sid && s.lifecycle.live())
            {
                self.app.close_session = Some(sid.clone());
                return OnCloseResponse::Ignore;
            }
            self.app.backends.remove(sid);
        }
        OnCloseResponse::Close
    }
    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        if response.hovered() && !response.dragged() {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.double_clicked()
            && let Tab::Terminal(sid) = tab
        {
            self.app.begin_rename(sid, RenameSurface::Pane);
        }
        if response.clicked()
            && let Tab::Terminal(sid) = tab
        {
            self.app.active_session = Some(sid.clone());
            self.app.send(Request::Focus {
                session: sid.clone(),
            });
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Tab, pane: egui_dock::NodePath) {
        if let Tab::Terminal(sid) = tab {
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            ui.separator();
        }
        self.app.new_terminal_menu(ui, Some(pane));
        ui.separator();
        if let Tab::Terminal(sid) = tab {
            if appearance::menu_item(ui, "Search scrollback", "Search", "").clicked() {
                self.app.search_session = Some(sid.clone());
                self.app.texts.remove(&format!("history:{sid}"));
                ui.close();
            }
            if appearance::menu_item(ui, "Clear saved scrollback", "Eraser", "").clicked() {
                self.app.send(Request::ClearHistory {
                    session: Some(sid.clone()),
                });
                self.app.texts.remove(&format!("history:{sid}"));
                ui.close();
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Image { path } => self.app.image_view(ui, path),
            Tab::Html { path } => self.app.html_view(ui, path),
            Tab::Player => self.app.player_view(ui),
            Tab::Diff { .. } => self.app.diff_view(ui, tab),
            Tab::Terminal(sid) => {
                let Some(session) = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .cloned()
                else {
                    ui.weak("Session record unavailable");
                    return;
                };
                let pane = self
                    .app
                    .pane_by_tab
                    .get(&Tab::Terminal(sid.clone()).key())
                    .copied();
                ui.spacing_mut().item_spacing.y = 2.0;
                let editing = self.app.renaming(sid, RenameSurface::Pane);
                let is_markdown = markdown::available(&session);
                let (response, close) = if is_markdown {
                    self.markdown_header(ui, &session, editing)
                } else {
                    appearance::pane_caption(
                        ui,
                        if editing { "" } else { &session.label },
                        self.app.active_session.as_ref() == Some(sid),
                        true,
                    )
                };
                if editing {
                    self.app.inline_rename(
                        ui,
                        sid,
                        RenameSurface::Pane,
                        egui::Rect::from_min_max(
                            response.rect.min + egui::vec2(8.0, 1.0),
                            response.rect.max
                                - egui::vec2(
                                    if close.is_some() && !is_markdown {
                                        28.0
                                    } else {
                                        8.0
                                    },
                                    1.0,
                                ),
                        ),
                    );
                }
                let closing = close.as_ref().is_some_and(|response| response.clicked());
                #[cfg(feature = "test-support")]
                if let Some(close) = &close {
                    diagnostics::record(ui.ctx(), &format!("editor-close:{sid}"), close.rect);
                    diagnostics::record(ui.ctx(), &format!("pane-close:{sid}"), close.rect);
                }
                if closing {
                    self.app.close_session = Some(sid.clone());
                }
                if response.clicked() && !closing && !editing {
                    self.app.active_session = Some(sid.clone());
                    self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                }
                if response.double_clicked() && !closing && !editing {
                    self.app.begin_rename(sid, RenameSurface::Pane);
                }
                appearance::context_menu(&response, |ui| {
                    if let Some(pane) = pane {
                        self.context_menu(ui, &mut Tab::Terminal(sid.clone()), pane);
                    }
                });
                if !session.lifecycle.live() {
                    self.app.backends.remove(sid);
                    ui.colored_label(
                        appearance::color(&self.app.theme.status_waiting),
                        format!(
                            "{:?} session — commands will not be run automatically",
                            session.lifecycle
                        ),
                    );
                    for a in self
                        .app
                        .state
                        .agents
                        .iter()
                        .filter(|a| a.session_id == *sid)
                    {
                        if let Some(resume) = &a.resume {
                            let text = resume.display();
                            ui.horizontal(|ui| {
                                ui.monospace(&text);
                                if ui.small_button("Copy resume").clicked() {
                                    ui.ctx().copy_text(text);
                                }
                            });
                        } else {
                            ui.weak(format!("{}: resume command unavailable", a.kind));
                        }
                    }
                    if ui.button("Open a fresh shell here").clicked() {
                        let _ = self.app.jobs.send(Job::rpc(
                            Request::Create {
                                project: session.project_id.clone(),
                                cwd: Some(session.cwd.clone()),
                                file: None,
                                line: None,
                                column: None,
                                editor: false,
                            },
                            After::Create(None),
                        ));
                    }
                    if session.truncated {
                        ui.weak("Some saved output was pruned or unavailable.");
                    }
                    let key = format!("history:{sid}");
                    if !self.app.texts.contains_key(&key) && self.app.loading.insert(key.clone()) {
                        let _ = self.app.jobs.send(Job::rpc(
                            Request::History {
                                session: sid.clone(),
                            },
                            After::Text(key.clone()),
                        ));
                    }
                    if let Some(text) = self.app.texts.get(&key) {
                        let lines = text.lines().collect::<Vec<_>>();
                        egui::ScrollArea::both().id_salt(key).show_rows(
                            ui,
                            18.0,
                            lines.len(),
                            |ui, range| {
                                for row in range {
                                    ui.monospace(lines[row]);
                                }
                            },
                        );
                    }
                    return;
                }
                if !self.app.connected {
                    ui.weak("Reconnecting to session daemon…");
                    return;
                }
                self.app.draw_unsaved_close_bar(ui, sid);
                if markdown::available(&session) {
                    self.markdown_view(ui, &session);
                } else {
                    self.terminal_view(ui, &session);
                }
            }
        }
    }
}

impl App {
    fn draw_unsaved_close_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some((target, ids, error)) = self.unsaved_close_prompt(sid) else {
            return;
        };
        let enabled = !self.editor_close_busy(&ids);
        let bar = appearance::unsaved_close_bar(
            ui,
            appearance::UnsavedCloseBar {
                theme: &self.theme,
                message: &error,
                enabled,
            },
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "Save and close", bar.save.rect);
            diagnostics::record(ui.ctx(), "Discard changes", bar.discard.rect);
            diagnostics::record(ui.ctx(), "Cancel", bar.cancel.rect);
        }
        if let Some(choice) = bar.choice() {
            self.apply_unsaved_close_choice(choice, target, ids);
        }
    }
}

impl Viewer<'_> {
    fn markdown_header(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        editing: bool,
    ) -> (egui::Response, Option<egui::Response>) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let header = appearance::markdown_header(
            ui,
            &session.label,
            self.app.active_session.as_ref() == Some(sid),
            editing,
            mode,
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "markdown-title", header.title.rect);
            diagnostics::record(ui.ctx(), "markdown-refresh", header.refresh.rect);
        }
        for (option, response) in header.modes {
            #[cfg(feature = "test-support")]
            {
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{}", option.label()),
                    response.rect,
                );
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{sid}:{}", option.label()),
                    response.rect,
                );
            }
            if response.clicked() {
                self.app.markdown.retain(sid).editor_focused = option != markdown::Mode::Preview;
                self.app.active_session = Some(sid.clone());
                self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                if option == markdown::Mode::default() {
                    self.app.preferences.markdown_modes.remove(sid);
                } else {
                    self.app
                        .preferences
                        .markdown_modes
                        .insert(sid.clone(), option);
                }
            }
        }
        if header.refresh.clicked() {
            self.app.markdown.refresh(ui.ctx());
        }
        (header.title, Some(header.close))
    }
    fn markdown_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let preview = self.app.markdown.retain(sid);
        if mode == markdown::Mode::Edit {
            preview.editor_focused = true;
        } else {
            preview.pointer_focus(ui);
        }
        let mut link = None;
        if mode != markdown::Mode::Edit {
            self.app.markdown.watch(markdown::Source::new(
                &self.app.state.session_paths(&self.app.paths, &session.id),
                session,
            ));
        }
        match mode {
            markdown::Mode::Edit => self.terminal_view(ui, session),
            markdown::Mode::Preview => link = self.app.markdown.retain(sid).show(ui, sid),
            markdown::Mode::Split => {
                let width = ui.available_width();
                egui::Panel::left(egui::Id::new(("markdown-editor", sid)))
                    .resizable(true)
                    .default_size(width * 0.5)
                    .size_range(80.0..=(width - 80.0).max(80.0))
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.terminal_view(ui, session));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| link = self.app.markdown.retain(sid).show(ui, sid));
            }
        }
        if let Some(link) = link {
            match link {
                markdown::Link::File(path) => self.app.terminal_action(
                    ui.ctx(),
                    session,
                    &services::Target::File(path, None, None),
                    FileAction::Open,
                ),
                markdown::Link::Web(url) => {
                    let _ = self.app.jobs.send(Job::Browser(url));
                }
            }
        }
    }
    fn terminal_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        self.app.visible_sessions.insert(sid.clone());
        if !self.app.backends.contains_key(sid) {
            let id = self.app.next_backend;
            self.app.next_backend += 1;
            let owner = self
                .app
                .state
                .generations
                .iter()
                .find(|g| g.owner.id == session.generation);
            let endpoint = owner
                .map(|g| g.owner.paths())
                .unwrap_or_else(|| self.app.paths.clone());
            let helper_result = owner
                .and_then(|g| g.helper.clone())
                .map(Ok)
                .unwrap_or_else(|| installation::attachment_helper(&self.app.state));
            let helper = match helper_result {
                Ok(p) => p,
                Err(e) => {
                    ui.label(e.to_string());
                    return;
                }
            };
            match TerminalBackend::new(
                id,
                ui.ctx().clone(),
                self.app.pty_tx.clone(),
                egui_term::BackendSettings {
                    shell: helper.to_string_lossy().into(),
                    args: vec![
                        "attach".into(),
                        sid.clone(),
                        endpoint.data.to_string_lossy().into(),
                        endpoint.runtime.to_string_lossy().into(),
                    ],
                    working_directory: None,
                },
            ) {
                Ok(b) => {
                    self.app.backends.insert(sid.clone(), b);
                    self.app.backend_ids.insert(id, sid.clone());
                }
                Err(e) => {
                    ui.colored_label(
                        appearance::color(&self.app.theme.status_failed),
                        format!("Cannot attach terminal: {e}"),
                    );
                    return;
                }
            }
        }
        let input_enabled = self.app.terminal_input_enabled(sid);
        let focused = input_enabled
            && self.app.active_session.as_ref() == Some(sid)
            && self
                .app
                .markdown
                .entries
                .get(sid)
                .is_none_or(|p| p.editor_focused);
        if focused {
            let count = ui
                .input_mut(|input| clipboard::take_image_paste(&mut input.events, input.modifiers));
            for _ in 0..count {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
            }
        }
        let backend = self.app.backends.get_mut(sid).unwrap();
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(self.app.state.settings.font_size),
        });
        let view = TerminalView::new(ui, backend)
            .external_links(true)
            .set_theme(egui_term::TerminalTheme::new(Box::new(
                egui_term::ColorPalette {
                    background: self.app.theme.terminal_background.clone(),
                    foreground: self.app.theme.terminal_foreground.clone(),
                    ..Default::default()
                },
            )))
            .set_focus(focused)
            .set_font(font)
            .set_size(ui.available_size());
        let response = ui.add_enabled(input_enabled, view);
        #[cfg(feature = "test-support")]
        {
            if session.kind == SessionKind::Shell {
                diagnostics::record(ui.ctx(), "terminal", response.rect);
            }
            diagnostics::record(ui.ctx(), &format!("terminal:{sid}"), response.rect);
            if session.kind == SessionKind::Editor {
                diagnostics::record(ui.ctx(), "editor-terminal", response.rect);
            }
        }
        if response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            if let Some(preview) = self.app.markdown.entries.get_mut(sid) {
                preview.editor_focused = true;
            }
            self.app.active_session = Some(sid.clone());
            self.app.send(Request::Focus {
                session: sid.clone(),
            });
        }
        let backend = self.app.backends.get(sid).unwrap();
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            let content = backend.last_content();
            let snapshot = (
                focused,
                content.grid.display_offset(),
                content.terminal_mode.bits(),
            );
            let key = egui::Id::new(("scroll-evidence", sid));
            if ui.ctx().data(|d| d.get_temp::<(bool, usize, u32)>(key)) != Some(snapshot) {
                eprintln!(
                    "Scroll evidence: session={sid} focused={} offset={} modes={}",
                    snapshot.0, snapshot.1, snapshot.2
                );
                ui.ctx().data_mut(|d| d.insert_temp(key, snapshot));
            }
        }
        let mouse_reporting = backend
            .last_content()
            .terminal_mode
            .intersects(egui_term::TerminalMode::MOUSE_MODE);
        let target = response.hover_pos().and_then(|pos| {
            backend.target_at(pos.x - response.rect.left(), pos.y - response.rect.top())
        });
        let selected = backend.selectable_content();
        let token = target.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), token);
        if !token.is_empty()
            && !self.app.targets.contains_key(&key)
            && self.app.loading.insert(key.clone())
        {
            let _ = self.app.jobs.send(Job::ResolveTarget(
                key.clone(),
                token.clone(),
                session.cwd.clone(),
            ));
        }
        let resolved = self.app.targets.get(&key).cloned().flatten();
        if let (Some(target), Some(resolved)) = (&target, &resolved) {
            if !ui.input(|i| i.pointer.any_down()) && !mouse_reporting {
                for rect in &target.rects {
                    let rect = rect.translate(response.rect.min.to_vec2());
                    ui.painter().with_clip_rect(response.rect).line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        egui::Stroke::new(1.0, appearance::color(&self.app.theme.accent)),
                    );
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if self.app.hover.as_ref().is_none_or(|(old, _)| old != &key) {
                    self.app.hover = Some((key.clone(), Instant::now()));
                }
                if self
                    .app
                    .hover
                    .as_ref()
                    .is_some_and(|(_, since)| since.elapsed() >= Duration::from_millis(400))
                    && self.app.hover_popup.is_none()
                {
                    let rect = target
                        .rects
                        .first()
                        .copied()
                        .unwrap_or(egui::Rect::ZERO)
                        .translate(response.rect.min.to_vec2());
                    self.app.hover_popup = Some((sid.clone(), resolved.clone(), rect));
                }
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        } else if response.contains_pointer() {
            self.app.hover = None;
        }
        if !mouse_reporting
            && response.clicked()
            && ui.input(|i| {
                if cfg!(target_os = "macos") {
                    i.modifiers.mac_cmd
                } else {
                    i.modifiers.ctrl
                }
            })
        {
            if let Some(target) = &resolved {
                self.app
                    .terminal_action(ui.ctx(), session, target, FileAction::Open);
            } else if !token.is_empty() {
                self.app.pending_target_action = Some((key.clone(), session.clone()));
            }
        }
        let menu_key = egui::Id::new(("terminal-menu-target", sid.as_str()));
        if response.secondary_clicked() {
            let text = if selected.trim().is_empty() {
                token.clone()
            } else {
                selected.clone()
            };
            let key = format!("target:{}:{}:{}", sid, session.cwd.display(), text);
            ui.ctx().data_mut(|d| d.insert_temp(menu_key, key.clone()));
            if !self.app.targets.contains_key(&key) && self.app.loading.insert(key.clone()) {
                let _ = self
                    .app
                    .jobs
                    .send(Job::ResolveTarget(key, text, session.cwd.clone()));
            }
        }
        appearance::context_menu(&response, |ui| {
            let command = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+Shift+"
            };
            ui.add_enabled_ui(!selected.is_empty(), |ui| {
                if appearance::menu_item(ui, "Copy", "Copy", &format!("{command}C")).clicked() {
                    ui.ctx().copy_text(selected.clone());
                    ui.close();
                }
            });
            if appearance::menu_item(ui, "Select all", "TextSelect", "").clicked() {
                if let Some(backend) = self.app.backends.get_mut(sid) {
                    backend.select_all();
                }
                ui.close();
            }
            if appearance::menu_item(ui, "Paste", "Clipboard", &format!("{command}V")).clicked() {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
                ui.close();
            }
            ui.separator();
            let pane = self
                .app
                .pane_by_tab
                .get(&Tab::Terminal(sid.clone()).key())
                .copied();
            self.app.new_terminal_menu(ui, pane);
            ui.separator();
            let key = ui
                .ctx()
                .data(|d| d.get_temp::<String>(menu_key))
                .unwrap_or_default();
            if let Some(Some(target)) = self.app.targets.get(&key).cloned() {
                appearance::target_header(ui, &target.display());
                if let Some(action) = file_actions::menu(ui, file_actions::target_menu(&target)) {
                    self.app.terminal_action(ui.ctx(), session, &target, action);
                }
            }
            if appearance::menu_item(ui, "Open file path…", "File", "").clicked() {
                self.app.path_text = selected.clone();
                self.app.open_path = true;
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(ui, "Search scrollback", "Search", "").clicked() {
                self.app.search_session = Some(sid.clone());
                self.app.texts.remove(&format!("history:{sid}"));
                ui.close();
            }
            if session.kind == SessionKind::Editor && !session.review {
                if appearance::menu_item(ui, "Save all", "Save", "⌘S").clicked() {
                    self.app.send(Request::EditorSave {
                        session: sid.clone(),
                    });
                    ui.close();
                }
                if appearance::menu_item(ui, "Compare disk", "FileDiff", "").clicked() {
                    self.app.send(Request::EditorCompare {
                        session: sid.clone(),
                    });
                    ui.close();
                }
            }
            if appearance::menu_item(ui, "Copy working directory", "Folder", "").clicked() {
                ui.ctx().copy_text(session.cwd.display().to_string());
                ui.close();
            }
            ui.separator();
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            if appearance::menu_item(ui, "Close session…", "X", "").clicked() {
                self.app.close_session = Some(sid.clone());
                ui.close();
            }
        });
        if let Some((owner, target, anchor)) = self.app.hover_popup.clone()
            && owner == *sid
        {
            let mut open = true;
            egui::Popup::from_response(&response)
                .id(egui::Id::new(("terminal-hover", sid.as_str())))
                .anchor(anchor)
                .open_bool(&mut open)
                .style(appearance::menu_style)
                .show(|ui| {
                    ui.set_max_width(440.0);
                    appearance::target_header(ui, &target.display());
                    if let Some(action) = file_actions::menu(ui, file_actions::target_menu(&target))
                    {
                        self.app.terminal_action(ui.ctx(), session, &target, action);
                    }
                });
            if !open {
                self.app.hover_popup = None;
                self.app.hover = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str) -> diff::DiffSpan {
        diff::DiffSpan {
            text: text.into(),
            rgb: [209, 211, 217],
            intra: diff::Intra::None,
        }
    }

    fn line(
        kind: diff::LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: &str,
    ) -> diff::DiffLine {
        diff::DiffLine {
            kind,
            old_no,
            new_no,
            spans: vec![span(text)],
        }
    }

    fn sample_line() -> diff::DiffLine {
        line(diff::LineKind::Insert, None, Some(1), "visible-diff-marker")
    }

    fn sample_doc() -> diff::DiffDocument {
        diff::DiffDocument {
            left_label: "Index".into(),
            right_label: "Working tree".into(),
            unified: vec![sample_line()],
            split: vec![],
        }
    }

    fn paint_diff_view(app: &mut App, ctx: &egui::Context, tab: &Tab) -> Vec<(egui::Pos2, String)> {
        let mut painted = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    // egui_dock tab body: no pane scrollbars, expand to the leaf,
                    // then the native viewer owns its own ScrollArea.
                    egui::ScrollArea::new([false, false]).show(ui, |ui| {
                        let available = ui.available_rect_before_wrap();
                        ui.expand_to_include_rect(available);
                        app.diff_view(ui, tab);
                    });
                },
            );
            painted = painted_text(&output.shapes);
            output.textures_delta.clear();
        }
        painted
    }

    fn paint_doc(doc: diff::DiffDocument, split: bool) -> Vec<(egui::Pos2, String)> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        if split {
            app.diff_split.insert(tab.key());
        }
        app.diffs.insert(tab.key(), Ok(doc));
        paint_diff_view(&mut app, &ctx, &tab)
    }

    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Pos2, String)> {
        fn walk(out: &mut Vec<(egui::Pos2, String)>, shape: &egui::Shape) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(out, shape);
                    }
                }
                egui::Shape::Text(text) => out.push((text.pos, text.galley.text().to_owned())),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&mut out, &clipped.shape);
        }
        out
    }

    fn require_text<'a>(
        painted: &'a [(egui::Pos2, String)],
        needle: &str,
    ) -> &'a (egui::Pos2, String) {
        painted
            .iter()
            .find(|(_, text)| text == needle)
            .unwrap_or_else(|| panic!("{needle} was not painted: {painted:?}"))
    }

    #[test]
    fn unified_diff_text_stays_in_the_viewport() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, text) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x),
            "code must start at the gutter, not centered or at x=0, got {pos:?} {text}"
        );
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "code galley must not include line numbers, got {text:?}"
        );
    }

    #[test]
    fn diff_view_paints_hunks_in_a_dock_pane() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, _) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x) && (0.0..500.0).contains(&pos.y),
            "hunk text must stay in the pane, got {pos:?}"
        );
    }

    #[test]
    fn equal_lines_share_a_gutter_x() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(8), Some(8), "x"),
                    line(
                        diff::LineKind::Equal,
                        Some(9),
                        Some(9),
                        "this-is-a-much-longer-equal-line",
                    ),
                ],
                split: vec![],
            },
            false,
        );
        let short = require_text(&painted, "x");
        let long = require_text(&painted, "this-is-a-much-longer-equal-line");
        assert!(
            (short.0.x - long.0.x).abs() < 1.0,
            "short and long lines must share a gutter, got {} vs {}",
            short.0.x,
            long.0.x
        );
    }

    #[test]
    fn delete_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![line(
                    diff::LineKind::Delete,
                    Some(100),
                    None,
                    "removed-line",
                )],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "100");
        let sign = require_text(&painted, "-");
        let code = require_text(&painted, "removed-line");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "minus must sit in its own column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
        assert!(
            code.0.x > sign.0.x,
            "code must start after the sign, got sign={} code={}",
            sign.0.x,
            code.0.x
        );
    }

    #[test]
    fn insert_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![line(diff::LineKind::Insert, None, Some(102), "added-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "102");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x,
            "plus must sit to the right of the new number, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn split_paints_one_number_per_side() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![],
                split: vec![diff::SplitRow {
                    left: Some(line(diff::LineKind::Delete, Some(5), None, "left-only")),
                    right: Some(line(diff::LineKind::Insert, None, Some(6), "right-only")),
                }],
            },
            true,
        );
        let left = require_text(&painted, "left-only");
        let right = require_text(&painted, "right-only");
        let old_no = require_text(&painted, "5");
        let new_no = require_text(&painted, "6");
        assert!(
            left.0.x < right.0.x,
            "split sides must not stack, got left={} right={}",
            left.0.x,
            right.0.x
        );
        assert!(
            old_no.0.x < left.0.x && old_no.0.x < 200.0,
            "old number must stay on the left gutter, got {old_no:?}"
        );
        assert!(
            new_no.0.x > left.0.x && new_no.0.x < right.0.x,
            "new number must stay on the right gutter, got old={} new={} left={} right={}",
            old_no.0.x,
            new_no.0.x,
            left.0.x,
            right.0.x
        );
    }

    #[test]
    fn diff_rows_use_font_height_not_item_spacing() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(1), Some(1), "row-a"),
                    line(diff::LineKind::Equal, Some(2), Some(2), "row-b"),
                ],
                split: vec![],
            },
            false,
        );
        let a = require_text(&painted, "row-a");
        let b = require_text(&painted, "row-b");
        let pitch = b.0.y - a.0.y;
        assert!(
            (12.0..22.0).contains(&pitch),
            "row pitch must be the font line height, not height+8 item_spacing, got {pitch}"
        );
    }

    #[test]
    fn equal_line_keeps_old_and_new_numbers_apart() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![line(diff::LineKind::Equal, Some(97), Some(97), "unchanged")],
                split: vec![],
            },
            false,
        );
        let numbers: Vec<_> = painted.iter().filter(|(_, text)| text == "97").collect();
        assert_eq!(
            numbers.len(),
            2,
            "unified equal lines paint old and new numbers separately, got {painted:?}"
        );
        let gap = (numbers[1].0.x - numbers[0].0.x).abs();
        assert!(
            gap > 8.0,
            "old and new 97 must be distinct columns, got {} and {}",
            numbers[0].0.x,
            numbers[1].0.x
        );
    }

    #[test]
    fn hunk_header_paints_no_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![line(diff::LineKind::Hunk, None, None, "@@ -3,2 +3,2 @@")],
                split: vec![],
            },
            false,
        );
        let header = require_text(&painted, "@@ -3,2 +3,2 @@");
        assert!(
            (40.0..200.0).contains(&header.0.x),
            "hunk text must align with code, got {header:?}"
        );
        assert!(
            painted.iter().all(|(_, text)| text != "3"),
            "hunk rows must not paint fake line numbers, got {painted:?}"
        );
    }

    #[test]
    fn five_digit_line_numbers_still_clear_the_sign() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                unified: vec![line(diff::LineKind::Insert, None, Some(10000), "wide-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "10000");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "5-digit numbers must not overflow into the sign column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn gutter_digit_columns_grow_with_line_numbers() {
        assert_eq!(diff_gutter_digits(std::iter::empty()), 4);
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(9999),
                Some(9999),
                "n",
            ))),
            4
        );
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(10000),
                Some(10000),
                "n",
            ))),
            5
        );
    }
    #[test]
    fn insertion_only_split_rows_keep_the_right_column() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "old".into(),
                right_label: "new".into(),
                unified: vec![],
                split: vec![
                    diff::SplitRow {
                        left: None,
                        right: Some(line(diff::LineKind::Insert, None, Some(1), "insert-only")),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Equal, Some(1), Some(2), "paired-left")),
                        right: Some(line(
                            diff::LineKind::Equal,
                            Some(1),
                            Some(2),
                            "paired-right",
                        )),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Delete, Some(2), None, "delete-only")),
                        right: None,
                    },
                ],
            },
            true,
        );
        assert_eq!(
            require_text(&painted, "insert-only").0.x,
            require_text(&painted, "paired-right").0.x
        );
        assert_eq!(
            require_text(&painted, "delete-only").0.x,
            require_text(&painted, "paired-left").0.x
        );
    }

    fn scroll_doc(
        ctx: &egui::Context,
        doc: &diff::DiffDocument,
        split: bool,
    ) -> (
        egui::scroll_area::ScrollAreaOutput<()>,
        Vec<egui::epaint::ClippedShape>,
    ) {
        let mut scroll = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                scroll = Some(paint_diff_document(
                    ui,
                    doc,
                    split,
                    DiffColors {
                        added: Color32::GREEN,
                        deleted: Color32::RED,
                        accent: Color32::BLUE,
                        text: Color32::WHITE,
                    },
                    "scroll-regression",
                ));
            },
        );
        output.textures_delta.clear();
        (scroll.unwrap(), output.shapes)
    }

    fn long_doc() -> diff::DiffDocument {
        let unified: Vec<_> = (1..=150)
            .map(|n| {
                line(
                    diff::LineKind::Equal,
                    Some(n),
                    Some(n),
                    &if n == 1 {
                        format!("{}END", "long-line-".repeat(30))
                    } else {
                        format!("short-{n}")
                    },
                )
            })
            .collect();
        let split = unified
            .iter()
            .map(|line| diff::SplitRow {
                left: Some(line.clone()),
                right: Some(line.clone()),
            })
            .collect();
        diff::DiffDocument {
            left_label: "old".into(),
            right_label: "new".into(),
            unified,
            split,
        }
    }

    #[test]
    fn long_split_lines_have_scroll_space_and_do_not_cross_their_column() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (scroll, _) = scroll_doc(&ctx, &doc, true);
        assert!(scroll.content_size.x > 3000.0);
        // Bring the end of the left column and beginning of the right into view.
        let mut state = scroll.state;
        state.offset.x = scroll.content_size.x / 2.0 - 400.0;
        state.store(&ctx, scroll.id);
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let long: Vec<_> = shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().ends_with("END")
                {
                    Some((shape.clip_rect, text))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(long.len(), 2);
        assert!(long[0].0.right() <= long[1].0.left() + 1.0);
        for (clip, text) in long {
            assert!(
                text.pos.x + text.galley.size().x <= clip.right() + 1.0
                    || clip.right() == scroll.inner_rect.right()
            );
        }
    }

    #[test]
    fn split_row_pitch_matches_the_virtualized_font_height() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let painted = painted_text(&shapes);
        let a = require_text(&painted, "short-2");
        let b = require_text(&painted, "short-3");
        let font = ctx.global_style().text_styles[&egui::TextStyle::Monospace].clone();
        let height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        assert!((b.0.y - a.0.y - height).abs() <= 1.0);
    }

    #[test]
    fn horizontal_scroll_reaches_the_end_of_the_right_split_line() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (scroll, _) = scroll_doc(&ctx, &doc, true);
        let mut state = scroll.state;
        state.offset.x = scroll.content_size.x - scroll.inner_rect.width();
        state.store(&ctx, scroll.id);
        let (scroll, shapes) = scroll_doc(&ctx, &doc, true);
        let end = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().ends_with("END") => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((end - scroll.inner_rect.right()).abs() <= 1.0);
    }

    #[test]
    fn horizontal_scroll_survives_vertical_virtualization_and_mode_switches() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        for split in [false, true] {
            scroll_doc(&ctx, &doc, split);
            let (before, _) = scroll_doc(&ctx, &doc, split);
            let width = before.content_size.x;
            let mut state = before.state;
            state.offset = egui::vec2(250.0, 900.0);
            state.store(&ctx, before.id);
            let (after, _) = scroll_doc(&ctx, &doc, split);
            assert_eq!(after.content_size.x, width);
            assert_eq!(after.state.offset, egui::vec2(250.0, 900.0));
        }
        let (unified, _) = scroll_doc(&ctx, &doc, false);
        let mut state = unified.state;
        state.offset = egui::vec2(100.0, 300.0);
        state.store(&ctx, unified.id);
        let (split, _) = scroll_doc(&ctx, &doc, true);
        assert_ne!(unified.id, split.id);
        assert_eq!(split.state.offset, egui::vec2(250.0, 900.0));
        let (unified, _) = scroll_doc(&ctx, &doc, false);
        assert_eq!(unified.state.offset, egui::vec2(100.0, 300.0));
    }

    #[test]
    fn refreshing_a_diff_invalidates_its_cached_width() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let (long, _) = scroll_doc(&ctx, &doc, false);
        let (short, _) = scroll_doc(&ctx, &sample_doc(), false);
        assert!(long.content_size.x > short.content_size.x * 2.0);
    }
}
