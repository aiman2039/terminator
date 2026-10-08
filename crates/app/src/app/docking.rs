use super::super::appearance;
use eframe::egui::{self};
use egui_dock::TabViewer;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
impl App {
    /// Strip-dock pane lookup, rebuilt every strip render (small dock; no cache).
    pub(crate) fn refresh_strip_pane_maps(&mut self, dock: &egui_dock::DockState<Tab>) {
        self.strip_pane_by_tab = dock
            .iter_all_tabs()
            .map(|(path, tab)| (tab.key(), path.node_path()))
            .collect();
        self.strip_pane_tabs = self
            .strip_pane_by_tab
            .values()
            .map(|path| {
                (
                    *path,
                    dock.leaf(*path)
                        .map(|leaf| leaf.tabs.clone())
                        .unwrap_or_default(),
                )
            })
            .collect();
    }

    pub(crate) fn apply_focus_strip_tab(&mut self, dock: &mut egui_dock::DockState<Tab>) {
        if let Some(tab) = self.focus_strip_tab.take()
            && let Some(path) = dock.find_tab(&tab)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    pub(crate) fn apply_add_strip_tab(
        &mut self,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
    ) {
        let Some((path, split)) = self.add_strip_tab.take() else {
            return;
        };
        let cwd = dock
            .leaf(path)
            .ok()
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .and_then(|tab| match tab {
                Tab::Terminal(id) => self
                    .state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.cwd.clone()),
                _ => None,
            })
            .or_else(|| self.selected_project().map(|p| p.path.clone()));
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: project.to_owned(),
                cwd,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            // Anchor even plain "new tab" on the clicked pane; otherwise the
            // tab lands in whichever strip leaf happened to be focused.
            After::StripAt(
                dock.leaf(path)
                    .map(|leaf| leaf.tabs.clone())
                    .unwrap_or_default(),
                split,
            ),
        ));
    }

    /// Clear a finished or cancelled pane drag, including its ghost snapshot.
    pub(crate) fn end_pane_drag(&mut self) {
        self.pane_drag = None;
        self.pane_drag_from_strip = false;
        self.pane_drag_snapshot.clear();
        self.strip_tab_hover = false;
        self.strip_new_tab_hover = false;
    }

    /// Drop zone within a hovered split leaf: the middle swaps or joins,
    /// while a band near an edge opens the dragged pane in a new split
    /// beside that leaf.
    pub(crate) fn pane_drop_zone(rect: egui::Rect, pos: egui::Pos2) -> PaneDropZone {
        let band = (rect.width().min(rect.height()) * 0.25).clamp(20.0, 96.0);
        let top = pos.y - rect.top();
        let bottom = rect.bottom() - pos.y;
        let left = pos.x - rect.left();
        let right = rect.right() - pos.x;
        if top <= band && top <= bottom && top <= left && top <= right {
            PaneDropZone::Above
        } else if bottom <= band && bottom <= top && bottom <= left && bottom <= right {
            PaneDropZone::Below
        } else if left <= band && left <= top && left <= bottom && left <= right {
            PaneDropZone::Left
        } else if right <= band && right <= top && right <= bottom && right <= left {
            PaneDropZone::Right
        } else {
            PaneDropZone::Center
        }
    }

    /// Complete a caption-initiated pane drag. Releasing over a split leaf of
    /// the previewed top-level tab lands the pane there (single panes swap,
    /// otherwise the dragged pane joins the leaf, including across tabs);
    /// releasing elsewhere cancels and switches back to the origin tab. The
    /// workspace strip runs earlier in the frame and consumes releases over
    /// its own tabs.
    pub(crate) fn finish_pane_drop(&mut self, ui: &mut egui::Ui, dock: &mut Workspace) {
        let Some(pane) = self.pane_drag.clone() else {
            return;
        };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.revert_drop_preview(dock);
            self.end_pane_drag();
            return;
        }
        let dragging = ui.input(|i| i.pointer.any_down());
        let released = ui.input(|i| i.pointer.any_released());
        if !dragging && !released {
            return;
        }
        let pos = ui.input(|i| i.pointer.interact_pos());
        let Some(pos) = pos else {
            if released {
                self.revert_drop_preview(dock);
                self.end_pane_drag();
            }
            return;
        };
        let target = dock
            .iter_leaves()
            .find(|(_, leaf)| leaf.rect.contains(pos))
            .map(|(path, leaf)| (path, leaf.rect));
        let zone = target.map(|(_, rect)| Self::pane_drop_zone(rect, pos));
        if dragging && !released {
            self.paint_landing_preview(ui, dock, &pane, target.map(|(path, _)| path), zone);
            // Hovering a strip tab interior previews the move-into outcome
            // at real size: wash the previewed tab's focused leaf, where a
            // release would land the pane.
            if target.is_none() && self.strip_tab_hover {
                let path = dock
                    .main_surface()
                    .focused_leaf()
                    .map(|node| egui_dock::NodePath {
                        surface: egui_dock::SurfaceIndex::main(),
                        node,
                    });
                let landed = path.filter(|path| {
                    dock.leaf(*path)
                        .map(|leaf| leaf.rect.width() > 1.0 && leaf.rect.height() > 1.0)
                        .unwrap_or(false)
                });
                if let Some(path) = landed {
                    #[cfg(feature = "test-support")]
                    if let Ok(leaf) = dock.leaf(path) {
                        diagnostics::record(ui.ctx(), "strip-drop-wash", leaf.rect);
                    }
                    self.paint_landing_preview(
                        ui,
                        dock,
                        &pane,
                        Some(path),
                        Some(PaneDropZone::Center),
                    );
                }
            }
            ui.ctx().request_repaint();
            return;
        }
        if released {
            if let Some((path, _)) = target {
                let group = dock.active.clone();
                let from_strip = self.pane_drag_from_strip;
                let moved = if from_strip {
                    match &pane {
                        Tab::Terminal(sid) => self.land_strip_shell_on_leaf(
                            dock,
                            sid,
                            &group,
                            path,
                            zone.unwrap_or(PaneDropZone::Center),
                        ),
                        _ => false,
                    }
                } else {
                    match zone {
                        Some(PaneDropZone::Center) | None => {
                            if dock.find_tab(&pane).is_some() {
                                // Same group: `move_pane_to_leaf` also focuses a
                                // drop back onto the pane's own leaf.
                                dock.move_pane_to_leaf(&pane, path)
                            } else {
                                dock.move_pane_to_group_leaf(&pane, &group, path)
                            }
                        }
                        Some(edge) => {
                            // `move_pane_to_split` focuses a lone pane dropped on
                            // an edge of its own leaf instead of splitting it.
                            edge.split().is_some_and(|split| {
                                dock.move_pane_to_split(&pane, &group, path, split)
                            })
                        }
                    }
                };
                if moved {
                    if let Tab::Terminal(sid) = &pane {
                        self.active_session = Some(sid.clone());
                        self.focus_tab = Some(pane.clone());
                    }
                    self.pane_index = None;
                    self.drop_preview_origin = None;
                } else {
                    self.revert_drop_preview(dock);
                }
            } else {
                self.revert_drop_preview(dock);
            }
            // A release the strip did not consume ends the drag here; the
            // post-dock checkout in `workspace_project` clears leftovers.
            self.end_pane_drag();
        }
    }

    /// Switch back to the tab a cancelled drag started from. Previewing never
    /// moves panes, so the origin group always still exists.
    pub(crate) fn revert_drop_preview(&mut self, dock: &mut Workspace) {
        if let Some((_, origin)) = self.drop_preview_origin.take()
            && dock.tabs.iter().any(|tab| tab.id == origin)
        {
            dock.active = origin;
        }
    }

    /// Landing preview for the hovered split leaf: a translucent accent wash
    /// over exactly where the dragged pane will land (the whole leaf, or the
    /// edge half a split drop would open), with a divider on the future
    /// split boundary. A center drop between two single panes exchanges
    /// them, so the source leaf is outlined as well.
    pub(crate) fn paint_landing_preview(
        &self,
        ui: &mut egui::Ui,
        dock: &Workspace,
        pane: &Tab,
        target: Option<egui_dock::NodePath>,
        zone: Option<PaneDropZone>,
    ) {
        let (Some(path), Some(zone)) = (target, zone) else {
            return;
        };
        let Ok(leaf) = dock.leaf(path) else {
            return;
        };
        let accent = appearance::color(&self.theme.accent);
        let wash = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36);
        let landing = zone.landing(leaf.rect);
        ui.painter().rect_filled(landing, 2, wash);
        ui.painter().rect_stroke(
            landing,
            2,
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Inside,
        );
        if zone != PaneDropZone::Center {
            // Divider where the new split boundary will appear.
            let divider = match zone {
                PaneDropZone::Above => [landing.left_bottom(), landing.right_bottom()],
                PaneDropZone::Below => [landing.left_top(), landing.right_top()],
                PaneDropZone::Left => [landing.right_top(), landing.right_bottom()],
                PaneDropZone::Right => [landing.left_top(), landing.left_bottom()],
                PaneDropZone::Center => return,
            };
            ui.painter()
                .line_segment(divider, egui::Stroke::new(2.0, accent));
        }
        // A center swap keeps every split in place and only exchanges two
        // panes: outline the other side and say so.
        let source = dock.find_tab(pane).map(egui_dock::TabPath::node_path);
        let mut swapping = false;
        if zone == PaneDropZone::Center
            && let Some(node) = source
            && node != path
            && let (Ok(from), Ok(to)) = (dock.leaf(node), dock.leaf(path))
            && from.tabs.len() == 1
            && to.tabs.len() == 1
        {
            swapping = true;
            ui.painter().rect_stroke(
                from.rect,
                2,
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            egui::pos2(landing.min.x + 8.0, landing.min.y + 6.0),
            egui::Align2::LEFT_TOP,
            if swapping {
                "Swap terminals"
            } else {
                zone.label()
            },
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220),
        );
    }

    pub(crate) fn apply_focus_tab(&mut self, dock: &mut Workspace) {
        if let Some(tab) = self.focus_tab.take()
            && let Some(path) = dock.find_tab(&tab)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    pub(crate) fn apply_add_tab(&mut self, project: &str, dock: &mut Workspace) {
        let Some((path, split)) = self.add_tab.take() else {
            return;
        };
        let cwd = dock
            .leaf(path)
            .ok()
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .and_then(|tab| match tab {
                Tab::Terminal(id) => self
                    .state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.cwd.clone()),
                Tab::Diff { cwd, .. } => Some(cwd.clone()),
                Tab::Image { path } | Tab::NativeEditor { path } => {
                    path.parent().map(PathBuf::from)
                }
                Tab::Browser { target, .. } => {
                    target.file().and_then(Path::parent).map(PathBuf::from)
                }
                Tab::Player => None,
                Tab::CommitLog { cwd } | Tab::Blame { cwd, .. } => Some(cwd.clone()),
            })
            .or_else(|| self.selected_project().map(|p| p.path.clone()));
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: project.to_owned(),
                cwd,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            // The leaf `+` always opens stacked inside that split; the
            // workspace-strip `+` (a new top-level tab) arrives through
            // `create_workspace_tab`, not here.
            After::CreateAt(
                dock.leaf(path)
                    .map(|leaf| leaf.tabs.clone())
                    .unwrap_or_default(),
                split,
            ),
        ));
    }

    /// Detach the queued pane into a fresh top-level tab at the end of the
    /// strip. Runs after the workspace dock is checked back in; unknown or
    /// already-moved panes are a silent no-op via the move primitive.
    pub(crate) fn apply_detach_pane(&mut self, dock: &mut Workspace) {
        let Some(pane) = self.detach_pane.take() else {
            return;
        };
        if dock
            .move_pane_to_new_group_at(&pane, dock.tabs.len())
            .is_some()
        {
            if let Tab::Terminal(sid) = &pane {
                self.active_session = Some(sid.clone());
                self.focus_tab = Some(pane);
            }
            self.pane_index = None;
            self.pending_layout_save = true;
        }
    }

    /// Panes allowed outside the app window. The browser mounts a native
    /// webview in the main window and the player is a global singleton
    /// view; everything else renders in any viewport.
    pub(crate) fn floatable(tab: &Tab) -> bool {
        !matches!(tab, Tab::Browser { .. } | Tab::Player)
    }

    /// Float the queued pane into its own OS window. Runs after the
    /// workspace dock is checked back in; panes that left the dock or
    /// cannot float are a silent no-op.
    pub(crate) fn apply_float_pane(&mut self, project: &str, dock: &mut Workspace) {
        let Some(pane) = self.float_pane.take() else {
            return;
        };
        if !Self::floatable(&pane) {
            return;
        }
        let Some(home) = dock.take_pane(&pane) else {
            return;
        };
        let viewport = egui::ViewportId::from_hash_of(format!("floating:{}", pane.key()));
        self.floating.push(FloatingPane {
            viewport,
            tab: Some(pane),
            home: (project.to_owned(), home),
        });
        self.pane_index = None;
        self.pending_layout_save = true;
    }

    /// Returns one floating pane to its workspace: the home tab when that
    /// still exists, else the project's active leaf.
    pub(crate) fn dock_back_floating(&mut self, project: String, home: String, tab: Tab) {
        self.layouts
            .entry(project)
            .or_insert_with(Workspace::empty)
            .dock_back(tab, &home);
        self.pane_index = None;
        self.pending_layout_save = true;
    }

    /// Returns every floating pane to its workspace, e.g. before the exit
    /// checkpoint so the saved layout stays complete.
    pub(crate) fn dock_back_all_floating(&mut self) {
        for pane in std::mem::take(&mut self.floating) {
            if let (Some(tab), (project, home)) = (pane.tab, pane.home) {
                self.dock_back_floating(project, home, tab);
            }
        }
    }

    /// Live window title for a floating pane, mirroring the tab captions.
    pub(crate) fn floating_title(&self, tab: &Tab) -> String {
        match tab {
            Tab::Terminal(sid) => {
                let label = self
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .map(|s| s.label.clone())
                    .unwrap_or_else(|| "Terminal".into());
                let presented = self.present_session(sid);
                if presented.attention.waiting() > 0 {
                    format!("● {label}")
                } else if presented.attention.failed > 0 {
                    format!("▲ {label}")
                } else {
                    label
                }
            }
            Tab::NativeEditor { path } => format!(
                "{}{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                if self.native_dirty(path) { " ●" } else { "" }
            ),
            Tab::Image { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Tab::Diff { path, staged, .. } => format!(
                "{} {}",
                if *staged { "Staged:" } else { "Diff:" },
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Tab::CommitLog { .. } => "Commit Log".into(),
            Tab::Blame { path, .. } => format!(
                "Blame {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Tab::Browser { .. } | Tab::Player => "Terminator".into(),
        }
    }

    /// Marks floated panes visible before the end-of-frame prunes. Floats
    /// paint after [`Self::paint_floating`], so without this their backends,
    /// images, and markdown previews would be dropped and re-attached every
    /// frame. Tested by `floated_panes_seed_visibility_before_prune`.
    pub(crate) fn seed_floating_visibility(&mut self) {
        for pane in &self.floating {
            match pane.tab.as_ref() {
                Some(Tab::Terminal(sid)) => {
                    self.visible_sessions.insert(sid.clone());
                    if self
                        .state
                        .sessions
                        .iter()
                        .find(|session| &session.id == sid)
                        .is_some_and(markdown::available)
                    {
                        self.markdown.retain(sid);
                    }
                }
                Some(Tab::Image { path }) => {
                    self.visible_images.insert(path.clone());
                }
                _ => {}
            }
        }
    }

    /// Renders every floating pane in its own OS window. A closed window
    /// docks its pane back instead of closing it.
    pub(crate) fn paint_floating(&mut self, ctx: &egui::Context) {
        if self.floating.is_empty() {
            return;
        }
        // The vec is checked out while viewports render so the viewer can
        // borrow the app; windows that stay open move back afterwards.
        let mut closed: Vec<(String, String, Tab)> = Vec::new();
        for mut pane in std::mem::take(&mut self.floating) {
            let Some(mut tab) = pane.tab.take() else {
                continue;
            };
            let title = self.floating_title(&tab);
            let mut close = false;
            ctx.show_viewport_immediate(
                pane.viewport,
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([960.0, 600.0])
                    .with_min_inner_size([420.0, 300.0]),
                |ui, _| {
                    close = ui.ctx().input(|i| i.viewport().close_requested());
                    // Fixture driver for window-close dock-back: the file's
                    // presence closes every float through the same path as
                    // the OS close button. Inert unless the env var is set.
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_TEST_CLOSE_FLOATING")
                        .is_some_and(|path| std::path::Path::new(&path).exists())
                    {
                        close = true;
                    }
                    crate::workspace_ui::Viewer {
                        app: self,
                        strip: false,
                    }
                    .ui(ui, &mut tab);
                },
            );
            if close {
                closed.push((pane.home.0, pane.home.1, tab));
            } else {
                pane.tab = Some(tab);
                self.floating.push(pane);
            }
        }
        for (project, home, tab) in closed {
            self.dock_back_floating(project, home, tab);
        }
    }

    /// Dock the queued pane back into its chosen sibling tab. Same
    /// checked-back-in contract as [`Self::apply_detach_pane`].
    pub(crate) fn apply_dock_back(&mut self, dock: &mut Workspace) {
        let Some((pane, dest)) = self.dock_back_pane.take() else {
            return;
        };
        if dock.move_pane_to_group(&pane, &dest) {
            if let Tab::Terminal(sid) = &pane {
                self.active_session = Some(sid.clone());
                self.focus_tab = Some(pane);
            }
            self.pane_index = None;
            self.pending_layout_save = true;
        }
    }

    pub(crate) fn paint_session_focus(&mut self, ui: &mut egui::Ui, dock: &Workspace) {
        if self.highlight_session != self.active_session {
            self.highlight_session = self.active_session.clone();
            self.highlight_since = Instant::now();
        }
        if let Some(sid) = &self.active_session
            && let Some(path) = dock.find_tab(&Tab::Terminal(sid.clone()))
            && let Ok(leaf) = dock.leaf(path.node_path())
        {
            ui.painter().rect_stroke(
                leaf.rect.shrink(1.0),
                2,
                appearance::focus_stroke(
                    appearance::color(&self.theme.accent),
                    self.highlight_since.elapsed(),
                ),
                egui::StrokeKind::Inside,
            );
        }
        if self.highlight_since.elapsed() < Duration::from_millis(1200) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}
