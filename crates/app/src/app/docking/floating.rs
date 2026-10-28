//! Floating pane windows: floating a pane out, painting each window's
//! dock in its own viewport, and docking layouts back when they close.

use super::super::center::PaintFloatDockInput;
use crate::*;

impl App {
    /// Panes allowed outside the app window. The browser mounts a native
    /// webview in the main window and the player is a global singleton
    /// view; everything else renders in any viewport.
    pub(crate) fn floatable(tab: &Tab) -> bool {
        !matches!(tab, Tab::Browser { .. } | Tab::Player)
    }

    /// Float the queued pane into its own OS window with a dock holding
    /// just that pane. Runs after the workspace dock is checked back in;
    /// panes that left the dock or cannot float are a silent no-op.
    pub(crate) fn apply_float_pane(&mut self, project: &str, dock: &mut Workspace) {
        let Some(pane) = self.float_pane.take() else {
            return;
        };
        if !Self::floatable(&pane) {
            return;
        }
        let home = if let Some(source) = self.pane_action_source.take() {
            (source.project == project)
                .then(|| dock.take_view(&source, &pane))
                .flatten()
        } else {
            dock.take_pane(&pane)
        };
        let Some(home) = home else {
            return;
        };
        let viewport = egui::ViewportId::from_hash_of(terminator_core::id());
        self.floating.push(FloatingWindow {
            viewport,
            dock: Some(egui_dock::DockState::new(vec![pane])),
            home: (project.to_owned(), home),
        });
        self.pane_index = None;
        self.pending_layout_save = true;
    }

    /// Returns one floating window's layout to its workspace: a lone pane
    /// rejoins its home leaf exactly like before, while a split layout is
    /// adopted whole as a new top-level tab so its structure survives.
    pub(crate) fn dock_back_window(
        &mut self,
        project: String,
        home: String,
        dock: egui_dock::DockState<Tab>,
    ) {
        let workspace = self.layouts.entry(project).or_insert_with(Workspace::empty);
        let mut tabs = dock
            .iter_all_tabs()
            .map(|(_, tab)| tab.clone())
            .collect::<Vec<_>>();
        if tabs.len() == 1 {
            // Length checked above; the pop cannot miss.
            if let Some(tab) = tabs.pop() {
                workspace.dock_back(tab, &home);
            }
        } else if !tabs.is_empty() {
            workspace.adopt_dock(dock, true);
        }
        self.pane_index = None;
        self.pending_layout_save = true;
    }

    /// Returns every floating window to its workspace, e.g. before the
    /// exit checkpoint so the saved layout stays complete.
    pub(crate) fn dock_back_all_floating(&mut self) {
        for window in std::mem::take(&mut self.floating) {
            if let (Some(dock), (project, home)) = (window.dock, window.home) {
                self.dock_back_window(project, home, dock);
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
            Tab::RevisionDiff { .. } | Tab::FileHistory { .. } => tab.git_title(),
            Tab::Vnc {
                label, host, port, ..
            } => {
                if !label.trim().is_empty() {
                    label.clone()
                } else if !host.trim().is_empty() {
                    format!("{host}:{port}")
                } else {
                    "VNC Viewer".into()
                }
            }
            Tab::Browser { .. } | Tab::Player => "Terminator".into(),
        }
    }

    /// Marks floated panes visible before the end-of-frame prunes. Floats
    /// paint after [`Self::paint_floating`], so without this their backends,
    /// images, and markdown previews would be dropped and re-attached every
    /// frame. Tested by `floated_panes_seed_visibility_before_prune`.
    pub(crate) fn seed_floating_visibility(&mut self) {
        for window in &self.floating {
            let Some(dock) = window.dock.as_ref() else {
                continue;
            };
            for (_, tab) in dock.iter_all_tabs() {
                match tab {
                    Tab::Terminal(sid) => {
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
                    Tab::Image { path } => {
                        self.visible_images.insert(path.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    /// Renders every floating window's dock in its own OS window. A closed
    /// window docks its whole layout back; an emptied one goes away.
    pub(crate) fn paint_floating(&mut self, ctx: &egui::Context) {
        if self.floating.is_empty() {
            return;
        }
        // Docks are checked out while viewports render so the viewer can
        // borrow the app; windows that stay open move back afterwards.
        let mut closed: Vec<(String, String, egui_dock::DockState<Tab>)> = Vec::new();
        let mut keep: Vec<FloatingWindow> = Vec::with_capacity(self.floating.len());
        for mut window in std::mem::take(&mut self.floating) {
            let Some(mut dock) = window.dock.take() else {
                keep.push(window);
                continue;
            };
            if dock.iter_all_tabs().next().is_none() {
                continue;
            }
            let viewport = window.viewport;
            let project = window.home.0.clone();
            let title = self.floating_title_for_dock(&mut dock);
            let mut close = false;
            ctx.show_viewport_immediate(
                viewport,
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
                    self.paint_float_dock(PaintFloatDockInput {
                        ui,
                        viewport,
                        project: &project,
                        dock: &mut dock,
                    });
                },
            );
            if close {
                closed.push((window.home.0, window.home.1, dock));
            } else if dock.iter_all_tabs().next().is_some() {
                window.dock = Some(dock);
                keep.push(window);
            }
            // An emptied window drops its dock and goes away with it.
        }
        self.floating = keep;
        self.apply_diff_replacement();
        for (project, home, dock) in closed {
            self.dock_back_window(project, home, dock);
        }
        // Queues for windows that closed without rendering are dead;
        // dropping them keeps one stuck request from blocking later ones.
        if let Some((viewport, _, _)) = &self.add_float_tab
            && !self.floating.iter().any(|w| &w.viewport == viewport)
        {
            self.add_float_tab = None;
        }
        if let Some((viewport, _)) = &self.focus_float_tab
            && !self.floating.iter().any(|w| &w.viewport == viewport)
        {
            self.focus_float_tab = None;
        }
    }

    /// Live window title for a floating dock, mirroring the tab captions.
    fn floating_title_for_dock(&self, dock: &mut egui_dock::DockState<Tab>) -> String {
        dock.main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| self.floating_title(tab))
            .unwrap_or_else(|| "Terminator".into())
    }
}
