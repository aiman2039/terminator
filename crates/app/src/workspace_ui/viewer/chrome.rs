use super::super::super::*;
use super::super::tab_menu::split_action;
use super::Viewer;

impl Viewer<'_> {
    pub(super) fn pane_trailing_controls(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
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
            self.pane_add_menu(ui, path);
        })
        .response
        .on_hover_text("New tab or split this pane");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-dropdown", response.rect);
        let _ = response;
    }

    /// New tab and split items, plus browser and VNC tabs in the main dock.
    fn pane_add_menu(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        for (label, direction) in [
            ("New tab", None),
            ("Split up", Some("up")),
            ("Split down", Some("down")),
            ("Split left", Some("left")),
            ("Split right", Some("right")),
        ] {
            let response = appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon: match direction {
                    Some("up") => "PanelTopClose",
                    Some("down") => "PanelBottomClose",
                    Some("left") => "PanelLeftClose",
                    Some("right") => "PanelRightClose",
                    _ => "Plus",
                },
                shortcut: &self.app.shortcut_label(split_action(direction)),
            });
            #[cfg(feature = "test-support")]
            diagnostics::record_ui(ui, label, response.rect);
            if response.clicked() {
                self.queue_add_tab(path, direction.map(str::to_owned));
                ui.close();
            }
        }
        if self.strip || self.window.is_some() {
            return;
        }
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New Browser Tab",
            icon: "Globe",
            shortcut: &self.app.shortcut_label(Action::NewBrowser),
        })
        .clicked()
        {
            self.app.open_new_browser_tab(None);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New VNC Viewer",
            icon: "Monitor",
            shortcut: &self.app.shortcut_label(Action::NewVnc),
        })
        .clicked()
        {
            self.app.open_new_vnc_tab(None);
            ui.close();
        }
    }

    pub(super) fn tab_title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        let file_name = |path: &std::path::Path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        match tab {
            Tab::Image { path } => file_name(path).into(),
            Tab::Browser { target, .. } => target.title().into(),
            Tab::Player => "Player".into(),
            Tab::Terminal(sid) => self.terminal_tab_title(sid),
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
            Tab::RevisionDiff { .. } | Tab::FileHistory { .. } => tab.git_title().into(),
            Tab::Vnc {
                label, host, port, ..
            } => if !label.trim().is_empty() {
                label.clone()
            } else if !host.trim().is_empty() {
                format!("{host}:{port}")
            } else {
                "VNC Viewer".into()
            }
            .into(),
        }
    }

    /// Split-pane headers and terminal-strip tabs share the presentation
    /// model: waiting attention first, then failures.
    fn terminal_tab_title(&self, sid: &str) -> egui::WidgetText {
        let label = self
            .app
            .state
            .sessions
            .iter()
            .find(|s| s.id == *sid)
            .map(|s| s.label.clone())
            .unwrap_or("Session".into());
        let presented = self.app.present_session(sid);
        if presented.attention.waiting() > 0 {
            egui::RichText::new(format!("● {label}"))
                .color(self.app.terminal_title_color(sid))
                .into()
        } else if presented.attention.failed > 0 {
            egui::RichText::new(format!("▲ {label}"))
                .color(self.app.terminal_title_color(sid))
                .into()
        } else {
            egui::RichText::new(label)
                .color(self.app.terminal_title_color(sid))
                .into()
        }
    }

    pub(super) fn paint_terminal_leading(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        tab: &mut Tab,
    ) {
        let Tab::Terminal(sid) = tab else {
            return;
        };
        let leading = self.app.tab_leading(sid);
        let mut cursor = rect.left();
        let mut paint = |icon: &str, tint: egui::Color32, spin: bool| {
            appearance::paint_status_icon(appearance::PaintStatusIconInput {
                ui,
                rect: egui::Rect::from_center_size(
                    egui::pos2(
                        cursor + appearance::TERMINAL_LEADING_SLOT / 2.0,
                        rect.center().y,
                    ),
                    egui::vec2(13.0, 13.0),
                ),
                icon,
                tint,
                spin,
            });
            cursor += appearance::TERMINAL_LEADING_SLOT;
        };
        if let Some(brand) = leading.brand {
            paint(brand, appearance::icon_color(), false);
        }
        if let Some((icon, tint, spin)) = leading.status {
            paint(icon, tint, spin);
        } else if let Some(kind) = leading.kind {
            paint(kind, self.app.terminal_title_color(sid), false);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("strip-tab-icon:{sid}"), rect);
    }

    pub(super) fn close_tab(
        &mut self,
        tab: &mut Tab,
        pane: egui_dock::NodePath,
    ) -> OnCloseResponse {
        if matches!(tab, Tab::Diff { .. }) && self.app.block_unsaved_diff(Some(&tab.key())) {
            return OnCloseResponse::Ignore;
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
        if let Tab::NativeEditor { path } = tab
            && self.app.native_dirty(path)
        {
            self.app.native_close_prompt = Some((path.clone(), self.close_issuer(pane)));
            return OnCloseResponse::Ignore;
        }
        if let Tab::Vnc { session, .. } = tab
            && !session.is_empty()
        {
            // Deferred: the workspace is checked out while the dock renders,
            // so deciding here would miss the copies that are still docked.
            // The drain disconnects only when no tab still shows it.
            self.app.pending_vnc_disconnect = Some(session.clone());
        }
        // Buffer pruning runs after the dock is checked back in and keeps
        // the buffer (including undo history) while another view exists.
        OnCloseResponse::Close
    }

    /// The dock passes the closing tab's owning leaf: the dialog and save
    /// completion close exactly the clicked copy instead of a focus sample
    /// or every twin.
    fn close_issuer(&self, pane: egui_dock::NodePath) -> Option<crate::native_editor::CloseIssuer> {
        match (self.window, self.project.as_deref(), self.tab.as_deref()) {
            (Some(viewport), _, _) => Some(crate::native_editor::CloseIssuer::Floating {
                viewport,
                node: Some(pane),
                delayed: false,
            }),
            (None, Some(project), Some(tab)) => Some(crate::native_editor::CloseIssuer::Docked {
                project: project.to_owned(),
                tab: tab.to_owned(),
                node: Some(pane),
                delayed: false,
            }),
            _ => None,
        }
    }

    pub(super) fn tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
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
        self.track_strip_tab_drag(tab, response);
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

    /// Dragging a strip tab starts inside this dock. Leaving the strip
    /// promotes it to a pane drag; releasing inside keeps the dock reorder.
    /// egui_dock floats the tab on its button id, then hands us a second
    /// interact id (`tab_id.with("dragged")`), so `drag_started` on this
    /// response never fires. Match that floating id, and a release that
    /// comes back through the tab button itself.
    fn track_strip_tab_drag(&mut self, tab: &Tab, response: &egui::Response) {
        let Tab::Terminal(sid) = tab else {
            return;
        };
        if !self.strip
            || !self
                .app
                .state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.kind == SessionKind::Shell)
        {
            return;
        }
        let floating = response
            .ctx
            .dragged_id()
            .is_some_and(|drag_id| response.id == drag_id.with("dragged"));
        if response.drag_started() || response.dragged() || response.drag_stopped() || floating {
            self.app.drag.strip_tab = Some(sid.clone());
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(&response.ctx, &format!("strip-tab:{sid}"), response.rect);
    }

    pub(super) fn tab_button_at(
        &mut self,
        tab: &mut Tab,
        path: egui_dock::TabPath,
        response: &egui::Response,
    ) -> bool {
        self.tab_button(tab, response);
        if !self.strip && self.window.is_none() {
            self.app.drag.drop_tabs.push((path, response.rect));
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(
            &response.ctx,
            &format!(
                "leaf-tab-at:{}:{}:{}",
                path.surface.0, path.node.0, path.tab.0
            ),
            response.rect,
        );
        let dragging = response.drag_started()
            || response.dragged()
            || response
                .ctx
                .dragged_id()
                .is_some_and(|id| response.id == id.with("dragged"));
        if !self.strip
            && self.window.is_none()
            && matches!(tab, Tab::NativeEditor { .. })
            && self.app.drag.pane.is_none()
            && dragging
            && let (Some(project), Some(group)) = (&self.project, &self.tab)
        {
            self.app.drag.pane = Some(tab.clone());
            self.app.drag.pane_source = Some(PaneSource {
                project: project.clone(),
                group: group.clone(),
                path,
            });
            self.app.drag.pane_snapshot.clear();
            return true;
        }
        false
    }
}
