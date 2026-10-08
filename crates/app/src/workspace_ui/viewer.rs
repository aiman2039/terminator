use super::super::*;
use super::header::filter_history_lines;
use super::tab_menu::split_action;
use super::tabs::snapshot_rows;
pub(crate) struct Viewer<'a> {
    pub(crate) app: &'a mut App,
    /// True when rendering the IDE strip dock instead of the main dock.
    /// Creation queues, pane maps, and focus targets switch docks; the
    /// strip always shows native tab bars, while the main dock shows
    /// them only on stacked leaves (nested tabs) and otherwise titles
    /// single panes with captions under the workspace strip.
    pub(crate) strip: bool,
    /// Docked project under render, if any: native close requests
    /// remember their issuing view. Floating windows pass none (the
    /// viewport identifies them instead).
    pub(crate) project: Option<String>,
    /// Top-level tab under render, if any: same-file twin tabs need it
    /// to tell the issuing copy apart.
    pub(crate) tab: Option<String>,
    /// Floating window under render, if any: pane actions (splits, focus,
    /// creation) route to this window's dock instead of the main dock.
    pub(crate) window: Option<egui::ViewportId>,
    /// Leaf currently rendering, reported by the dock before each tab
    /// body: native close requests remember their issuing pane. Starts
    /// empty for direct (non-dock) renders such as floating panes.
    pub(crate) render_path: Option<egui_dock::NodePath>,
}
impl Viewer<'_> {
    /// Queue tab creation in the dock under render: strip, floating
    /// window, or main dock. Floating splits land in the issuing window.
    fn queue_add_tab(&mut self, pane: egui_dock::NodePath, split: Option<String>) {
        if self.strip {
            self.app.add_strip_tab = Some((pane, split));
        } else if let Some(viewport) = self.window {
            self.app.add_float_tab = Some((viewport, pane, split));
        } else {
            self.app.add_tab = Some((pane, split));
        }
    }
    /// Focus a tab in the dock under render.
    fn queue_focus_tab(&mut self, tab: Tab) {
        if self.strip {
            self.app.focus_strip_tab = Some(tab);
        } else if let Some(viewport) = self.window {
            self.app.focus_float_tab = Some((viewport, tab));
        } else {
            self.app.focus_tab = Some(tab);
        }
    }
    /// Leaf path for a tab key in the dock under render, if any.
    pub(super) fn pane_for(&self, key: &str) -> Option<egui_dock::NodePath> {
        if self.strip {
            self.app.strip_pane_by_tab.get(key).copied()
        } else if self.window.is_some() {
            self.app.float_pane_by_tab.get(key).copied()
        } else {
            self.app.pane_by_tab.get(key).copied()
        }
    }
}
impl TabViewer for Viewer<'_> {
    fn set_render_path(&mut self, path: egui_dock::NodePath) {
        self.render_path = Some(path);
    }
    fn on_add(&mut self, path: egui_dock::NodePath) {
        if self.strip {
            self.app.add_strip_tab = Some((path, None));
        } else if let Some(viewport) = self.window {
            self.app.add_float_tab = Some((viewport, path, None));
        } else {
            self.app.add_tab = Some((path, None));
        }
    }
    type Tab = Tab;
    fn show_tab_bar(&self, path: egui_dock::NodePath) -> bool {
        // Stacked leaves read as nested tabs: a leaf holding more than
        // one pane shows its bar even in the main dock, whose single
        // panes keep the workspace strip as their only tab row. Floating
        // windows read their own maps; a lone floated pane keeps the OS
        // window title as its only tab row, like before.
        if self.strip {
            return true;
        }
        let tabs = if self.window.is_some() {
            &self.app.float_pane_tabs
        } else {
            &self.app.pane_tabs
        };
        tabs.get(&path).is_some_and(|tabs| tabs.len() > 1)
    }
    fn trailing_controls_width(&self) -> f32 {
        let actions = if self.strip {
            appearance::strip_terminal_actions_width() + 4.0
        } else {
            0.0
        };
        actions + 28.0
    }
    fn trailing_controls(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        // The dock's own style can set a taller interact size; pin it so every
        // toolbar control resolves to the same square and centers share a row.
        ui.spacing_mut().interact_size =
            egui::vec2(appearance::TERMINAL_BUTTON, appearance::TERMINAL_BUTTON);
        ui.spacing_mut().item_spacing.x = 4.0;
        if self.strip {
            self.strip_terminal_actions(ui, path);
        }
        #[cfg(feature = "test-support")]
        {
            let rect = ui.max_rect();

            diagnostics::record(
                ui.ctx(),
                "pane-plus",
                rect.translate(egui::vec2(-24.0, 0.0)),
            );
        }
        let response = appearance::icon_menu_button(ui, "ChevronDown", |ui| {
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
                    &self.app.shortcut_label(split_action(direction)),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), label, response.rect);
                if response.clicked() {
                    self.queue_add_tab(path, direction.map(str::to_owned));
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
            Tab::Browser { target, .. } => target.title().into(),
            Tab::Player => "Player".into(),
            Tab::Terminal(sid) => {
                let label = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .map(|s| s.label.clone())
                    .unwrap_or("Session".into());
                // Split-pane headers and terminal-strip tabs share the
                // presentation model: waiting attention first, then failures.
                let presented = self.app.present_session(sid);
                if presented.attention.waiting() > 0 {
                    format!("● {label}").into()
                } else if presented.attention.failed > 0 {
                    format!("▲ {label}").into()
                } else {
                    label.into()
                }
            }
            Tab::Diff { path, staged, .. } => format!(
                "{} {}",
                if *staged { "Staged:" } else { "Diff:" },
                path.file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
            Tab::NativeEditor { path } => format!(
                "{}{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                if self.app.native_dirty(path) {
                    " ●"
                } else {
                    ""
                }
            )
            .into(),
            Tab::CommitLog { .. } => "Commit Log".into(),
            Tab::Blame { path, .. } => format!(
                "Blame {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
        }
    }
    fn tab_leading_width(&self, tab: &Tab) -> f32 {
        match tab {
            Tab::Terminal(sid) => self.app.tab_leading(sid).width(),
            _ => 0.0,
        }
    }
    fn paint_tab_leading(&mut self, ui: &mut egui::Ui, rect: egui::Rect, tab: &mut Tab) {
        let Tab::Terminal(sid) = tab else {
            return;
        };
        let leading = self.app.tab_leading(sid);
        let mut cursor = rect.left();
        let mut paint = |icon: &str, tint: egui::Color32, spin: bool| {
            appearance::paint_status_icon(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(
                        cursor + appearance::TERMINAL_LEADING_SLOT / 2.0,
                        rect.center().y,
                    ),
                    egui::vec2(13.0, 13.0),
                ),
                icon,
                tint,
                spin,
            );
            cursor += appearance::TERMINAL_LEADING_SLOT;
        };
        if let Some(brand) = leading.brand {
            paint(brand, appearance::ICON_COLOR, false);
        }
        if let Some((icon, tint, spin)) = leading.status {
            paint(icon, tint, spin);
        } else if let Some(kind) = leading.kind {
            paint(kind, appearance::ICON_COLOR, false);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("strip-tab-icon:{sid}"), rect);
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
        if let Tab::NativeEditor { path } = tab {
            if self.app.native_dirty(path) {
                self.app.native_close_prompt = Some(path.clone());
                return OnCloseResponse::Ignore;
            }
            self.app.native_docs.remove(path);
        }
        OnCloseResponse::Close
    }
    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        if let Tab::Terminal(sid) = tab {
            let presented = self.app.present_session(sid);
            response.clone().on_hover_ui(|ui| {
                ui.label(presented.diagnostics(now()));
            });
        }
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
        // Dragging a strip tab starts inside this dock. Leaving the strip
        // promotes it to a pane drag; releasing inside keeps the dock reorder.
        // egui_dock floats the tab on its button id, then hands us a second
        // interact id (`tab_id.with("dragged")`), so `drag_started` on this
        // response never fires. Match that floating id, and a release that
        // comes back through the tab button itself.
        if self.strip
            && let Tab::Terminal(sid) = tab
            && self
                .app
                .state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.kind == SessionKind::Shell)
        {
            let floating = response
                .ctx
                .dragged_id()
                .is_some_and(|drag_id| response.id == drag_id.with("dragged"));
            if response.drag_started() || response.dragged() || response.drag_stopped() || floating
            {
                self.app.strip_tab_drag = Some(sid.clone());
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(&response.ctx, &format!("strip-tab:{sid}"), response.rect);
        }
        // Main-dock stacked tab buttons, so fixtures can switch the
        // nested tabs of a split leaf by recorded name.
        #[cfg(feature = "test-support")]
        if !self.strip {
            diagnostics::record(
                &response.ctx,
                &format!("leaf-tab:{}", tab.key()),
                response.rect,
            );
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Tab, pane: egui_dock::NodePath) {
        if let Tab::Terminal(sid) = tab {
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            ui.separator();
        }
        self.app
            .new_terminal_menu(ui, Some(pane), self.strip, self.window);
        ui.separator();
        // Detach, dock-back, and float ops reshape main-dock top-level
        // tabs; a floating window owns a single dock with no tab layer,
        // so they stay hidden there.
        if !self.strip && self.window.is_none() {
            // Detach tears this pane off into a fresh top-level tab;
            // dock-back returns it to a sibling tab. Both queue while
            // the workspace is checked out and apply once it is back.
            let stacked = self
                .app
                .pane_tabs
                .get(&pane)
                .is_some_and(|tabs| tabs.len() > 1);
            let detachable = stacked || !self.app.dock_back_targets.is_empty();
            let floatable = crate::App::floatable(tab);
            if detachable {
                if appearance::menu_item(ui, "Detach to new tab", "PanelsTopLeft", "").clicked() {
                    self.app.detach_pane = Some(tab.clone());
                    ui.close();
                }
                let targets = self.app.dock_back_targets.clone();
                if !targets.is_empty() {
                    let _ = appearance::text_menu_button(ui, "Move to tab", |ui| {
                        for (id, label, icon) in targets {
                            if appearance::menu_item(ui, &label, icon, "").clicked() {
                                self.app.dock_back_pane = Some((tab.clone(), id));
                                ui.close();
                            }
                        }
                    });
                }
            }
            if floatable && appearance::menu_item(ui, "Float window", "ExternalLink", "").clicked()
            {
                self.app.float_pane = Some(tab.clone());
                ui.close();
            }
            if detachable || floatable {
                ui.separator();
            }
        }
        if let Tab::Terminal(sid) = tab {
            let shell = self
                .app
                .state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.kind == SessionKind::Shell);
            if self.strip
                && shell
                && appearance::menu_item(
                    ui,
                    "Move to main pane",
                    "ArrowUp",
                    &self.app.shortcut_label("move_to_main"),
                )
                .clicked()
            {
                self.app.queue_strip_move(sid);
                ui.close();
            }
            // Main/strip moves have no meaning inside a floating window.
            if !self.strip
                && shell
                && self.window.is_none()
                && self.app.preferences.ide_mode
                && appearance::menu_item(
                    ui,
                    "Move to lower pane",
                    "ArrowDown",
                    &self.app.shortcut_label("move_to_strip"),
                )
                .clicked()
            {
                self.app.queue_main_to_strip(sid);
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "Search scrollback",
                "Search",
                &self.app.shortcut_label("search_scrollback"),
            )
            .clicked()
            {
                self.app.open_scrollback_search(ui.ctx(), sid);
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "Clear saved scrollback",
                "Eraser",
                &self.app.shortcut_label("clear_scrollback"),
            )
            .clicked()
            {
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
            Tab::Browser { id, target } => {
                let key = id.clone();
                self.app.browser_view(ui, key, target);
            }
            Tab::Player => {
                self.app.open_player();
                ui.close();
            }
            Tab::Diff { .. } => self.app.diff_view(ui, tab),
            Tab::NativeEditor { path } => self.app.native_editor_view(
                ui,
                path,
                self.project.as_deref(),
                self.tab.as_deref(),
                self.render_path,
            ),
            Tab::CommitLog { .. } => self.app.commit_log_view(ui, tab),
            Tab::Blame { .. } => self.app.blame_view(ui, tab),
            Tab::Terminal(sid) => {
                let Some(session) = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .cloned()
                else {
                    // The daemon pruned the record (a plain shell or editor
                    // without an agent resume handle). Drop the stale tab
                    // instead of leaving a dead placeholder behind.
                    self.app.queue_unavailable_tab_close(sid);
                    return;
                };
                let editing = self.app.renaming(sid, RenameSurface::Pane);
                if self.strip {
                    // Native tab bars title strip panes, so there is no
                    // caption chrome (close, drag, and menus all live on the
                    // tab). Only an in-progress rename needs a row of its own.
                    if editing {
                        let slot = ui.allocate_response(
                            egui::vec2(ui.available_width(), 26.0),
                            egui::Sense::hover(),
                        );
                        self.app
                            .inline_rename(ui, sid, RenameSurface::Pane, slot.rect);
                    }
                } else {
                    let pane = self.pane_for(&Tab::Terminal(sid.clone()).key());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let is_markdown = markdown::available(&session);
                    // A lone pane's name is already the workspace tab. Keep the
                    // drag and close row, but don't paint the title again.
                    // A lone floated pane shows the OS window title instead.
                    let lone = if self.window.is_some() {
                        self.app
                            .float_pane_tabs
                            .values()
                            .map(Vec::len)
                            .sum::<usize>()
                            == 1
                    } else {
                        self.app
                            .pane_index
                            .as_ref()
                            .is_some_and(|index| index.tabs == 1)
                    };
                    let branch = self.app.branch_at(&session.cwd);
                    let git_tip = match &branch {
                        Some(name) => format!("{name}\nOpen Git"),
                        None => "Open Git".into(),
                    };
                    let vertical_tip = self.app.action_tip("Split vertically", "split_right");
                    let horizontal_tip = self.app.action_tip("Split horizontally", "split_down");
                    let (response, close, actions) = if is_markdown {
                        let header = self.markdown_header(ui, &session, editing);
                        (header.0, header.1, None)
                    } else {
                        let leading = self.app.tab_leading(sid);
                        let bar = appearance::terminal_bar(
                            ui,
                            appearance::TerminalBarSpec {
                                title: if editing || lone { "" } else { &session.label },
                                active: self.app.active_session.as_ref() == Some(sid),
                                branch: branch.as_deref(),
                                status: self.app.terminal_status_color(&session),
                                git_tip: &git_tip,
                                // No leaf tab bar on a lone pane, so its `+`
                                // lives here instead. Hidden without a leaf
                                // to avoid dead clicks.
                                stack_tip: pane.is_some().then_some("New tab in this split"),
                                vertical_tip: &vertical_tip,
                                horizontal_tip: &horizontal_tip,
                                brand: leading.brand,
                                status_icon: leading.status.map(|(icon, _, _)| icon),
                                status_tint: leading.status.map(|(_, tint, _)| tint),
                                spin: leading.status.is_some_and(|(_, _, spin)| spin),
                                kind: if leading.status.is_none() {
                                    leading.kind
                                } else {
                                    None
                                },
                            },
                        );
                        (
                            bar.bar,
                            Some(bar.close),
                            Some((bar.git, bar.stack, bar.split_vertical, bar.split_horizontal)),
                        )
                    };
                    #[cfg(feature = "test-support")]
                    if !is_markdown && !editing {
                        // Recorded even for lone panes (whose title the
                        // workspace tab already shows) so fixtures can
                        // address their caption menus.
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-caption:{}", session.label),
                            response.rect,
                        );
                        // The bar paints no title for lone panes (see the
                        // `title` argument above), so only titled panes
                        // record one: fixtures can tell title duplication.
                        if !lone {
                            diagnostics::record(
                                ui.ctx(),
                                &format!("pane-title:{}", session.label),
                                response.rect,
                            );
                        }
                    }
                    let controls_left = actions
                        .as_ref()
                        .map(|(git, _, _, _)| git.rect.left() - 4.0)
                        .unwrap_or_else(|| {
                            response.rect.right()
                                - if close.is_some() && !is_markdown {
                                    28.0
                                } else {
                                    8.0
                                }
                        });
                    if editing {
                        self.app.inline_rename(
                            ui,
                            sid,
                            RenameSurface::Pane,
                            egui::Rect::from_min_max(
                                egui::pos2(response.rect.min.x + 8.0, response.rect.min.y + 1.0),
                                egui::pos2(controls_left, response.rect.bottom() - 1.0),
                            ),
                        );
                    }
                    let closing = close.as_ref().is_some_and(eframe::egui::Response::clicked);
                    let git_clicked = actions.as_ref().is_some_and(|(git, _, _, _)| git.clicked());
                    let stack_clicked = actions.as_ref().is_some_and(|(_, stack, _, _)| {
                        stack.as_ref().is_some_and(eframe::egui::Response::clicked)
                    });
                    let split_vertical = actions
                        .as_ref()
                        .is_some_and(|(_, _, split, _)| split.clicked());
                    let split_horizontal = actions
                        .as_ref()
                        .is_some_and(|(_, _, _, split)| split.clicked());
                    let on_control = close.as_ref().is_some_and(eframe::egui::Response::hovered)
                        || actions
                            .as_ref()
                            .is_some_and(|(git, stack, vertical, horizontal)| {
                                git.hovered()
                                    || stack.as_ref().is_some_and(eframe::egui::Response::hovered)
                                    || vertical.hovered()
                                    || horizontal.hovered()
                            });
                    #[cfg(feature = "test-support")]
                    if let Some(close) = &close {
                        diagnostics::record(ui.ctx(), &format!("editor-close:{sid}"), close.rect);
                        diagnostics::record(ui.ctx(), &format!("pane-close:{sid}"), close.rect);
                    }
                    #[cfg(feature = "test-support")]
                    if let Some((git, stack, vertical, horizontal)) = &actions {
                        diagnostics::record(ui.ctx(), &format!("pane-git:{sid}"), git.rect);
                        if let Some(stack) = stack {
                            diagnostics::record(ui.ctx(), &format!("pane-stack:{sid}"), stack.rect);
                        }
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-split-vertical:{sid}"),
                            vertical.rect,
                        );
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-split-horizontal:{sid}"),
                            horizontal.rect,
                        );
                    }
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("pane-drag:{sid}"), response.rect);
                    // Caption drag starts a pane move. The drop lands on another
                    // split leaf (rearrange) or a workspace strip tab (move
                    // across top-level tabs); clicks still focus as before.
                    // Floating windows opt out: cross-window drops are
                    // unsupported, so a drag starting here could never land.
                    if response.drag_started()
                        && self.window.is_none()
                        && !editing
                        && !closing
                        && !on_control
                    {
                        self.app.pane_drag = Some(Tab::Terminal(sid.clone()));
                        // Snapshot once: the ghost reuses it every frame instead
                        // of re-reading the live grid while it scrolls.
                        self.app.pane_drag_snapshot = self
                            .app
                            .backends
                            .get(sid)
                            .map(snapshot_rows)
                            .unwrap_or_default();
                    }
                    if self.app.pane_drag.as_ref() == Some(&Tab::Terminal(sid.clone()))
                        && response.hovered()
                        && ui.input(|i| i.pointer.any_down())
                    {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                    }
                    if closing {
                        self.app.close_session = Some(sid.clone());
                    }
                    if git_clicked {
                        self.app.active_session = Some(sid.clone());
                        self.queue_focus_tab(Tab::Terminal(sid.clone()));
                        self.app.show_git_sidebar();
                    }
                    if split_vertical || split_horizontal || stack_clicked {
                        self.app.active_session = Some(sid.clone());
                        self.queue_focus_tab(Tab::Terminal(sid.clone()));
                    }
                    if let Some(pane) = pane {
                        // The caption `+` queues the same leaf-anchored
                        // request as the leaf tab bar `+`: a stacked tab in
                        // this split, never a new top-level tab. Floating
                        // splits land in the issuing window's dock.
                        if stack_clicked {
                            self.queue_add_tab(pane, None);
                        }
                        if split_vertical {
                            self.queue_add_tab(pane, Some("right".into()));
                        }
                        if split_horizontal {
                            self.queue_add_tab(pane, Some("down".into()));
                        }
                    }
                    if response.clicked() && !closing && !editing && !git_clicked && !on_control {
                        self.app.active_session = Some(sid.clone());
                        self.queue_focus_tab(Tab::Terminal(sid.clone()));
                    }
                    if response.double_clicked() && !closing && !editing && !on_control {
                        self.app.begin_rename(sid, RenameSurface::Pane);
                    }
                    appearance::context_menu(&response, |ui| {
                        if let Some(pane) = pane {
                            self.context_menu(ui, &mut Tab::Terminal(sid.clone()), pane);
                        }
                    });
                }
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
                        let pane = self.pane_for(&Tab::Terminal(sid.clone()).key());
                        if let (Some(viewport), Some(pane)) = (self.window, pane) {
                            // A fresh shell for a dead floated session stays
                            // in its window instead of the main dock.
                            self.app.add_float_tab = Some((viewport, pane, None));
                        } else {
                            let _ = self.app.jobs.send(Job::rpc(
                                Request::Create {
                                    project: session.project_id.clone(),
                                    cwd: Some(session.cwd.clone()),
                                    file: None,
                                    line: None,
                                    column: None,
                                    editor: false,
                                },
                                if self.strip {
                                    After::Strip
                                } else {
                                    After::Create(None)
                                },
                            ));
                        }
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
                    let sid_key = sid.clone();
                    let query = self
                        .app
                        .history_filter
                        .entry(sid_key.clone())
                        .or_default()
                        .clone();
                    ui.horizontal(|ui| {
                        ui.add(
                            appearance::singleline(
                                self.app.history_filter.entry(sid_key.clone()).or_default(),
                            )
                            .id(egui::Id::new(("history-filter", sid_key)))
                            .hint_text("Filter saved scrollback")
                            .desired_width(220.0),
                        );
                        if ui.small_button("✕").on_hover_text("Back").clicked() {
                            self.app.search_session = None;
                            self.app.history_filter.remove(sid);
                        }
                    });
                    if let Some(text) = self.app.texts.get(&key) {
                        let lines = filter_history_lines(text, &query);
                        if !query.is_empty() {
                            ui.monospace(format!("{} matching lines", lines.len()));
                        }
                        egui::ScrollArea::both().id_salt(key).show_rows(
                            ui,
                            18.0,
                            lines.len(),
                            |ui, range| {
                                for row in range {
                                    if let Some(line) = lines.get(row) {
                                        ui.monospace(*line);
                                    }
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
