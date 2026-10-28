use super::*;

struct FloatingCloseInput<'a> {
    path: &'a Path,
    viewport: egui::ViewportId,
    node: Option<egui_dock::NodePath>,
    delayed: bool,
}

struct DockedCloseInput<'a> {
    path: &'a Path,
    project: &'a str,
    tab: &'a str,
    node: Option<egui_dock::NodePath>,
    delayed: bool,
}

/// View that asked for a deferred native close, captured when the
/// command starts. Plain closes remove only this view; every other
/// copy of the file stays open.
#[derive(Clone, Debug)]
pub(crate) enum CloseIssuer {
    /// A docked pane: project, top-level tab, and leaf recorded up
    /// front, so a later focus move or a same-file twin tab cannot
    /// redirect the close. `delayed` marks a save-deferred `:wq`: the
    /// issuing view may be gone by the time the save settles, and then
    /// the close must take nothing instead of another copy.
    Docked {
        project: String,
        tab: String,
        node: Option<egui_dock::NodePath>,
        delayed: bool,
    },
    /// A pane docked in a floating window: the window plus the leaf
    /// recorded up front. Plain closes remove only the issuing copy;
    /// `delayed` marks a save-deferred `:wq` (exact pane or no-op).
    Floating {
        viewport: egui::ViewportId,
        node: Option<egui_dock::NodePath>,
        delayed: bool,
    },
}

/// Dirty-editor dialog decision for one path, applied by
/// [`App::apply_native_close_choice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeCloseChoice {
    Discard,
    SaveClose,
}

impl CloseIssuer {
    /// Mark an issuer as save-deferred (`:wq` queued at save start,
    /// closing frames later): exact issuing pane or no-op.
    pub(super) fn deferred(mut self) -> Self {
        match &mut self {
            Self::Docked { delayed, .. } | Self::Floating { delayed, .. } => {
                *delayed = true;
            }
        }
        self
    }
}

impl App {
    /// Issuing view for a close requested from a file view: the floating
    /// window it renders in, else the docked project/tab/leaf under
    /// render. Captured now (not at drain time) so later focus moves
    /// cannot redirect the close.
    pub(super) fn close_issuer(
        ui: &egui::Ui,
        project: Option<&str>,
        tab: Option<&str>,
        node: Option<egui_dock::NodePath>,
    ) -> Option<CloseIssuer> {
        let viewport = ui.ctx().viewport_id();
        if viewport != egui::ViewportId::ROOT {
            Some(CloseIssuer::Floating {
                viewport,
                node,
                delayed: false,
            })
        } else {
            match (project, tab) {
                (Some(project), Some(tab)) => Some(CloseIssuer::Docked {
                    project: project.to_owned(),
                    tab: tab.to_owned(),
                    node,
                    delayed: false,
                }),
                _ => None,
            }
        }
    }

    /// Close a native tab and drop its buffer. Callers confirm dirty tabs first.
    /// `force` (`:q!`, `:qa!`, explicit discard) closes every view of the
    /// file. A plain close removes only its issuing view (`issuer`):
    /// one docked copy in the issuing project, or the issuing copy of the
    /// floating window; every other copy stays open with its buffer.
    ///
    /// Callers rendering inside the workspace checkout
    /// (`native_editor_view`) must defer through `pending_native_close`:
    /// the workspace is removed from `layouts` while it renders, so a
    /// direct close finds no tab, drops the buffer, and the tab springs
    /// back on the next frame.
    pub(crate) fn close_native_tab(
        &mut self,
        path: &Path,
        force: bool,
        issuer: Option<CloseIssuer>,
    ) {
        if force {
            self.close_every_native_copy(path);
        } else {
            match issuer {
                Some(CloseIssuer::Docked {
                    project,
                    tab,
                    node,
                    delayed,
                }) => self.close_docked_native_copy(DockedCloseInput {
                    path,
                    project: &project,
                    tab: &tab,
                    node,
                    delayed,
                }),
                Some(CloseIssuer::Floating {
                    viewport,
                    node,
                    delayed,
                }) => self.close_floating_native_copy(FloatingCloseInput {
                    path,
                    viewport,
                    node,
                    delayed,
                }),
                // Not view-issued (modal, quit flows): legacy removal of
                // every docked copy.
                None => {
                    for workspace in self.layouts.values_mut() {
                        while Self::remove_one_docked_copy(workspace, path) {}
                    }
                }
            }
        }
        self.forget_closed_native(path);
        self.save_layouts();
    }

    /// `:q!`: every docked and floating copy of the file.
    fn close_every_native_copy(&mut self, path: &Path) {
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        for workspace in self.layouts.values_mut() {
            while Self::remove_one_docked_copy(workspace, path) {}
        }
        for window in &mut self.floating {
            if let Some(dock) = window.dock.as_mut() {
                while Self::remove_copy_from_float_dock(dock, &target, None, false) {}
            }
        }
        self.drop_empty_floats();
    }

    /// One copy in the issuing tab: the recorded pane first, then that
    /// tab's focused copy. Same-file twin tabs and other projects keep
    /// theirs.
    fn close_docked_native_copy(&mut self, input: DockedCloseInput<'_>) {
        let DockedCloseInput {
            path,
            project,
            tab,
            node,
            delayed,
        } = input;
        // A delayed `:wq` naming its pane closes exactly that pane: with
        // the issuing view gone, every fallback below would take a copy
        // that never asked to close.
        let strict = delayed && node.is_some();
        let removed = self.close_copy_in_project(DockedCloseInput {
            path,
            project,
            tab,
            node,
            delayed: strict,
        });
        if !removed && !strict {
            // Project gone since the request queued: take one copy
            // anywhere rather than dropping the close.
            for workspace in self.layouts.values_mut() {
                if Self::remove_one_docked_copy(workspace, path) {
                    break;
                }
            }
        }
    }

    /// The issuing project's part of a docked close. `delayed` here is
    /// already the strict flag. Returns whether a copy was removed.
    fn close_copy_in_project(&mut self, input: DockedCloseInput<'_>) -> bool {
        let DockedCloseInput {
            path,
            project,
            tab,
            node,
            delayed: strict,
        } = input;
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        let Some(workspace) = self.layouts.get_mut(project) else {
            return false;
        };
        let mut removed = workspace
            .tabs
            .iter_mut()
            .find(|tab_state| tab_state.id == *tab)
            .is_some_and(|tab_state| Self::remove_copy_from_tab(tab_state, &target, node, strict));
        if !removed && !strict {
            // Issuing tab replaced since the request queued: one copy in
            // the project, focused first.
            removed = Self::remove_one_docked_copy(workspace, path);
        }
        workspace.drop_empty_tabs();
        removed
    }

    /// Only the issuing copy in the issuing window; docked views and
    /// other windows keep the file.
    fn close_floating_native_copy(&mut self, input: FloatingCloseInput<'_>) {
        let FloatingCloseInput {
            path,
            viewport,
            node,
            delayed,
        } = input;
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        let strict = delayed && node.is_some();
        if let Some(window) = self.floating.iter_mut().find(|w| w.viewport == viewport)
            && let Some(dock) = window.dock.as_mut()
        {
            Self::remove_copy_from_float_dock(dock, &target, node, strict);
        }
        self.drop_empty_floats();
    }

    /// Drop the buffer and pending prompts once no view of `path` is left.
    fn forget_closed_native(&mut self, path: &Path) {
        // Split views (`:split` / `:vsplit` / `:tabnew`) share the one
        // buffer: dropping it while another view stays open would reload
        // from disk on the next frame and lose unsaved changes.
        if !self.all_native_tab_paths().contains(&path.to_owned()) {
            self.native_docs.remove(path);
        }
        if self
            .native_close_prompt
            .as_ref()
            .is_some_and(|(prompt, _)| prompt == path)
        {
            self.native_close_prompt = None;
        }
        if self.native_close_after_save.as_deref() == Some(path) {
            self.native_close_after_save = None;
            self.native_close_after_save_issuer = None;
        }
    }

    /// Run deferred native closes after the workspace is checked back into
    /// `layouts`. Call sites render outside the checkout (modals, quit
    /// flows) keep calling `close_native_tab` directly. Deferred `:split`
    /// / `:vsplit` / `:tabnew` drain here too: the workspace is checked
    /// out while the file view renders, so layout changes wait for it.
    /// Finished goto jumps drain here as well: opening a cross-file
    /// target needs the originating project, which is missing while the
    /// workspace is checked out for rendering.
    pub(crate) fn drain_pending_native_close(&mut self) {
        let pending = std::mem::take(&mut self.pending_native_close);
        for (path, force, issuer) in &pending {
            self.close_native_tab(path, *force, issuer.clone());
        }
        let splits = std::mem::take(&mut self.pending_native_splits);
        for (viewport, path, split) in splits {
            self.split_native_view(viewport, &path, split);
        }
        let jumps = std::mem::take(&mut self.pending_goto);
        for jump in jumps {
            self.apply_goto_jump(jump);
        }
        if let Some(force) = self.pending_quit_all.take() {
            self.drain_quit_all(force);
        }
    }

    /// `:qa` takes every view including floating ones; without `!` a
    /// dirty buffer refuses the whole quit and reports on each.
    fn drain_quit_all(&mut self, force: bool) {
        let paths = self.all_native_tab_paths();
        if !force {
            let dirty: Vec<PathBuf> = paths
                .iter()
                .filter(|p| self.native_dirty(p))
                .cloned()
                .collect();
            if !dirty.is_empty() {
                for path in &dirty {
                    if let Some(doc) = self.native_docs.get_mut(path) {
                        doc.save_error = Some("Unsaved changes (add ! to override)".into());
                    }
                }
                return;
            }
        }
        for path in &paths {
            self.close_native_tab(path, true, None);
        }
    }

    /// Apply a dirty-editor dialog choice for one path. Discard closes the
    /// issuing view when the prompt names one (scoped like `:q`, never
    /// every copy) and keeps legacy removal for flow-issued prompts.
    /// Save-and-close defers the issuing pane through the save so the
    /// settled close lands on the view that asked instead of every
    /// docked copy — or instead of leaving a float open.
    pub(crate) fn apply_native_close_choice(
        &mut self,
        path: &Path,
        choice: NativeCloseChoice,
        issuer: Option<CloseIssuer>,
    ) {
        match choice {
            NativeCloseChoice::Discard => match issuer {
                Some(issuer) => self.close_native_tab(path, false, Some(issuer)),
                None => self.close_native_tab(path, true, None),
            },
            NativeCloseChoice::SaveClose => {
                self.native_close_after_save = Some(path.to_owned());
                self.native_close_after_save_issuer = issuer.map(CloseIssuer::deferred);
                if let Some(doc) = self.native_docs.get_mut(path) {
                    doc.force_save = false;
                    doc.start_save(&self.services.clone());
                }
            }
        }
    }

    pub(crate) fn native_close_modal(&mut self, ctx: &egui::Context) {
        let Some((path, issuer)) = self.native_close_prompt.clone() else {
            return;
        };
        // Tab-close prompts render as an in-pane bar inside the issuing
        // view; the modal would steal input from the rest of the GUI.
        if issuer.is_some() {
            return;
        }
        if !self.native_dirty(&path) {
            self.close_native_tab(&path, false, issuer);
            return;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut open = true;
        let mut save_close = false;
        let mut discard = false;
        let mut cancel = false;
        self.popups
            .window(ctx, "Unsaved native changes?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("{name} has unsaved changes."));
                ui.horizontal(|ui| {
                    if ui.button("Save and close").clicked() {
                        save_close = true;
                    }
                    if ui.button("Discard changes").clicked() {
                        discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if discard {
            self.apply_native_close_choice(&path, NativeCloseChoice::Discard, issuer);
        } else if save_close {
            self.apply_native_close_choice(&path, NativeCloseChoice::SaveClose, issuer);
        } else if !open {
            // Dismissing the prompt is the quit's Cancel. Sparkle keeps
            // waiting if we only hide the prompt.
            self.cancel_app_quit();
        }
        if !open {
            self.native_close_prompt = None;
        }
    }
}
