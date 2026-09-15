//! Workspace and terminal rendering.
use super::*;

impl App {
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
                    .on_hover_text("Settings");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "settings", settings.rect);
                if settings.clicked() {
                    self.open_settings();
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
                                Some(Tab::Diff { path, .. }) | Some(Tab::Image { path }) => (
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned(),
                                    if matches!(primary, Some(Tab::Image { .. })) {
                                        "FileImage"
                                    } else {
                                        "FileDiff"
                                    },
                                    None,
                                ),
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
                            egui::Image::new(icons::source("X"))
                                .tint(appearance::ICON_COLOR)
                                .paint_at(
                                    ui,
                                    egui::Rect::from_center_size(
                                        close_rect.center(),
                                        egui::vec2(16.0, 16.0),
                                    ),
                                );
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
        for (label, split) in [
            ("New tab", None),
            ("Split up", Some("up")),
            ("Split down", Some("down")),
            ("Split left", Some("left")),
            ("Split right", Some("right")),
        ] {
            let icon = match split {
                Some("up") => "PanelTopClose",
                Some("down") => "PanelBottomClose",
                Some("left") => "PanelLeftClose",
                Some("right") => "PanelRightClose",
                _ => "Plus",
            };
            if appearance::menu_item(ui, label, icon, "").clicked() {
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
                    Tab::Diff { path, .. } | Tab::Image { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
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
            if ui.selectable_label(split, "Split").clicked() {
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
                let rows = if split {
                    doc.split.len()
                } else {
                    doc.unified.len()
                };
                let height = ui.text_style_height(&egui::TextStyle::Monospace).max(16.0);
                let colors = DiffColors {
                    added: appearance::color(&self.theme.git_added),
                    deleted: appearance::color(&self.theme.git_deleted),
                    accent: appearance::color(&self.theme.accent),
                    text: appearance::color(&self.theme.text),
                };
                let doc = doc.clone();
                egui::ScrollArea::both()
                    .id_salt(&key)
                    .show_rows(ui, height, rows, |ui, range| {
                        for index in range {
                            if split {
                                let width = ui.available_width() / 2.0;
                                ui.horizontal(|ui| {
                                    paint_diff_side(
                                        ui,
                                        egui::vec2(width, height),
                                        doc.split[index].left.as_ref(),
                                        colors,
                                    );
                                    paint_diff_side(
                                        ui,
                                        egui::vec2(width, height),
                                        doc.split[index].right.as_ref(),
                                        colors,
                                    );
                                });
                            } else {
                                paint_diff_line(
                                    ui,
                                    DiffPaint {
                                        width: ui.available_width(),
                                        height,
                                        line: &doc.unified[index],
                                        unified: true,
                                        colors,
                                    },
                                );
                            }
                        }
                    });
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

struct DiffPaint<'a> {
    width: f32,
    height: f32,
    line: &'a diff::DiffLine,
    unified: bool,
    colors: DiffColors,
}

fn paint_diff_side(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    line: Option<&diff::DiffLine>,
    colors: DiffColors,
) {
    ui.allocate_ui(size, |ui| {
        if let Some(line) = line {
            paint_diff_line(
                ui,
                DiffPaint {
                    width: size.x,
                    height: size.y,
                    line,
                    unified: false,
                    colors,
                },
            );
        }
    });
}

fn paint_diff_line(ui: &mut egui::Ui, paint: DiffPaint<'_>) {
    let DiffPaint {
        width,
        height,
        line,
        unified,
        colors,
    } = paint;
    let DiffColors {
        added,
        deleted,
        accent,
        text,
    } = colors;
    let (bg, sign) = match line.kind {
        diff::LineKind::Insert => (tint(added, 40), "+"),
        diff::LineKind::Delete => (tint(deleted, 40), "-"),
        diff::LineKind::Hunk => (tint(accent, 24), " "),
        diff::LineKind::Equal => (Color32::TRANSPARENT, " "),
    };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width.max(1.0), height), egui::Sense::hover());
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
    let gutter = format!(
        "{:>4} {:>4} {} ",
        line.old_no.map(|n| n.to_string()).unwrap_or_default(),
        line.new_no.map(|n| n.to_string()).unwrap_or_default(),
        if unified { sign } else { " " }
    );
    job.append(
        &gutter,
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    for span in &line.spans {
        let intra = match (line.kind, span.intra) {
            (diff::LineKind::Insert, diff::Intra::Change) => tint(added, 90),
            (diff::LineKind::Delete, diff::Intra::Change) => tint(deleted, 90),
            _ => Color32::TRANSPARENT,
        };
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: if line.kind == diff::LineKind::Hunk {
                    accent
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
                font_id: font,
                color: text,
                ..Default::default()
            },
        );
    }
    ui.put(
        rect,
        egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend),
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
            Tab::Image { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
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
            self.app
                .markdown
                .watch(markdown::Source::new(&self.app.paths, session));
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
            let helper = match installation::attachment_helper(&self.app.state) {
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
                        self.app.paths.data.to_string_lossy().into(),
                        self.app.paths.runtime.to_string_lossy().into(),
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
        let input_enabled = !self.app.picker_active
            && !self.app.settings_open
            && !self.app.add_project
            && !self.app.notice_detail_modal_open()
            && (self.app.close_session.is_none() || self.app.idle_close_pending.is_some())
            && !self.app.editor_close_sessions.contains(sid)
            && (self.app.close_workspace.is_none() || self.app.idle_close_pending.is_some())
            && self.app.rename_session.is_none()
            && !self.app.open_path
            && self.app.search_session.is_none();
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
                if let Some(action) = file_actions::menu(
                    ui,
                    matches!(target, services::Target::File(..)),
                    matches!(target, services::Target::Url(..)),
                    None,
                ) {
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
                    if let Some(action) = file_actions::menu(
                        ui,
                        matches!(target, services::Target::File(..)),
                        matches!(target, services::Target::Url(..)),
                        None,
                    ) {
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
