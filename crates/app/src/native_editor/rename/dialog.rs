//! The rename window: input, server wait, preview, and apply stages.
use super::*;

/// Buttons pressed in the rename window this frame.
#[derive(Default)]
struct RenameActions {
    request: bool,
    apply: bool,
    retry: bool,
}

/// Diff and error colors for the preview.
#[derive(Clone, Copy)]
struct RenameColors {
    added: Color32,
    removed: Color32,
    context: Color32,
}

impl App {
    pub(crate) fn show_native_rename_dialog(&mut self, ctx: &egui::Context) {
        self.lsp.poll();
        let Some(mut dialog) = self.native_rename.take() else {
            return;
        };
        self.adopt_rename_answer(&mut dialog);
        if let Some(stage) = self.settle_rename_stage(&mut dialog.stage) {
            // A finished apply closes the dialog.
            if matches!(dialog.stage, RenameStage::Applying { .. })
                && matches!(stage, RenameStage::Input)
            {
                return;
            }
            dialog.stage = stage;
        }
        let Some(actions) = self.rename_window(ctx, &mut dialog) else {
            return;
        };
        self.apply_rename_actions(&mut dialog, actions);
        self.native_rename = Some(dialog);
    }

    /// The server's edits for the name this dialog asked for.
    fn adopt_rename_answer(&mut self, dialog: &mut RenameDialog) {
        if matches!(dialog.stage, RenameStage::WaitingServer)
            && let Some(result) = self.lsp.take_rename()
            && result.from == dialog.path
            && result.new_name == dialog.draft.trim()
        {
            dialog.stage = match result.edits {
                Ok(edits) if !edits.is_empty() => self.prepare_stage(&dialog.root, edits),
                Ok(_) => RenameStage::Error("The language server found no rename edits.".into()),
                Err(error) => RenameStage::Error(error),
            };
        }
    }

    fn prepare_stage(&self, root: &Path, edits: Vec<native_lsp::RenameFileEdit>) -> RenameStage {
        match self.start_rename_prepare(root, edits) {
            Ok(rx) => RenameStage::Preparing(rx),
            Err(error) => RenameStage::Error(error),
        }
    }

    /// The next stage once a preview or apply worker answers.
    fn settle_rename_stage(&mut self, stage: &mut RenameStage) -> Option<RenameStage> {
        match stage {
            RenameStage::Preparing(rx) => self.settle_rename_preview(rx),
            RenameStage::Applying { rx, .. } => self.settle_rename_apply(rx),
            _ => None,
        }
    }

    fn settle_rename_preview(
        &self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RenameResult<PreparedRename>>,
    ) -> Option<RenameStage> {
        match rx.try_recv() {
            Ok(Ok(prepared)) if !prepared.files.is_empty() => {
                Some(match self.rename_open_docs_are_clean(&prepared) {
                    Ok(()) => RenameStage::Preview(prepared),
                    Err(error) => RenameStage::Error(error),
                })
            }
            Ok(Ok(_)) => Some(RenameStage::Error(
                "The rename would not change any file.".into(),
            )),
            Ok(Err(error)) => Some(RenameStage::Error(error)),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Some(RenameStage::Error("Rename preview worker stopped.".into()))
            }
        }
    }

    fn settle_rename_apply(
        &mut self,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RenameResult<Vec<PathBuf>>>,
    ) -> Option<RenameStage> {
        match rx.try_recv() {
            Ok(result) => Some(match result {
                Ok(paths) => self.finish_rename_apply(&paths),
                Err(error) => RenameStage::Error(error),
            }),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => Some(RenameStage::Error(
                "Rename apply worker stopped; check the affected files.".into(),
            )),
        }
    }

    /// Reload clean buffers the rename rewrote; keep edited ones for
    /// review.
    fn finish_rename_apply(&mut self, paths: &[PathBuf]) -> RenameStage {
        let mut retained_dirty = false;
        for path in paths {
            retained_dirty |= self.reload_renamed_doc(path);
        }
        self.lsp.invalidate_paths(paths);
        self.info = Some(if retained_dirty {
            "Rename applied, but an edited buffer was kept for review.".into()
        } else {
            format!("Renamed symbol in {} file(s).", paths.len())
        });
        RenameStage::Input
    }

    /// Returns true when the open buffer had edits and was kept.
    fn reload_renamed_doc(&mut self, path: &Path) -> bool {
        let Some(doc) = self.native_docs.get_mut(path) else {
            return false;
        };
        if doc.dirty() {
            doc.save_error = Some("Rename changed the file on disk while this buffer had edits. Review it before saving.".into());
            return true;
        }
        doc.doc = None;
        doc.load_rx = None;
        doc.loading = false;
        doc.load_error = None;
        false
    }

    /// Draw the window. `None` when the user closed it.
    fn rename_window(
        &mut self,
        ctx: &egui::Context,
        dialog: &mut RenameDialog,
    ) -> Option<RenameActions> {
        let applying = matches!(dialog.stage, RenameStage::Applying { .. });
        let mut open = true;
        let mut actions = RenameActions::default();
        let colors = RenameColors {
            added: appearance::color(&self.theme.git_added),
            removed: appearance::color(&self.theme.status_failed),
            context: appearance::color(&self.theme.secondary),
        };
        let window = self.popups.window(ctx, "Rename symbol").collapsible(false);
        let window = if applying {
            window
        } else {
            window.open(&mut open)
        };
        window.show(ctx, |ui| {
            actions = Self::rename_body(ui, dialog, colors);
        });
        open.then_some(actions)
    }

    fn rename_body(
        ui: &mut egui::Ui,
        dialog: &mut RenameDialog,
        colors: RenameColors,
    ) -> RenameActions {
        let mut actions = RenameActions::default();
        ui.weak(dialog.path.display().to_string());
        match &mut dialog.stage {
            RenameStage::Input => actions.request = Self::rename_input(ui, &mut dialog.draft),
            RenameStage::WaitingServer => {
                ui.spinner();
                ui.weak("Waiting for the language server…");
            }
            RenameStage::Preparing(_) => {
                ui.spinner();
                ui.weak("Checking files and preparing the preview…");
            }
            RenameStage::Preview(prepared) => {
                actions.apply = Self::rename_preview(ui, prepared, colors);
            }
            RenameStage::Applying { .. } => {
                ui.spinner();
                ui.weak("Applying the reviewed changes…");
            }
            RenameStage::Error(error) => {
                ui.colored_label(colors.removed, error.as_str());
                actions.retry = ui.button("Edit name and retry").clicked();
            }
        }
        actions
    }

    /// Name field and Preview button; returns whether to ask the server.
    fn rename_input(ui: &mut egui::Ui, draft: &mut String) -> bool {
        let mut request = false;
        ui.horizontal(|ui| {
            ui.label("New name");
            let field = ui.text_edit_singleline(draft);
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), "rename-name", field.rect);
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                request = true;
            }
        });
        let preview = ui.button("Preview changes");
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "rename-preview", preview.rect);
        if preview.clicked() {
            request = true;
        }
        request
    }

    /// Per-file diffs and the Apply button; returns whether to apply.
    fn rename_preview(ui: &mut egui::Ui, prepared: &PreparedRename, colors: RenameColors) -> bool {
        ui.label(format!(
            "{} edits in {} files. Review before applying.",
            prepared.edit_count,
            prepared.files.len()
        ));
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                for file in &prepared.files {
                    Self::rename_file_diff(ui, &prepared.root, file, colors);
                }
            });
        let apply_button = ui.button("Apply rename");
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "rename-apply", apply_button.rect);
        apply_button.clicked()
    }

    fn rename_file_diff(ui: &mut egui::Ui, root: &Path, file: &PreparedFile, colors: RenameColors) {
        let relative = file.path.strip_prefix(root).unwrap_or(&file.path);
        egui::CollapsingHeader::new(format!(
            "{} ({} edits)",
            relative.display(),
            file.edit_count
        ))
        .default_open(true)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in file.diff.lines() {
                let color = if line.starts_with('+') {
                    colors.added
                } else if line.starts_with('-') {
                    colors.removed
                } else {
                    colors.context
                };
                ui.label(egui::RichText::new(line).monospace().color(color));
            }
        });
    }

    fn apply_rename_actions(&mut self, dialog: &mut RenameDialog, actions: RenameActions) {
        let RenameActions {
            request,
            apply,
            retry,
        } = actions;
        if retry {
            dialog.stage = RenameStage::Input;
        }
        if request {
            dialog.stage = self.request_rename_stage(dialog);
        }
        if apply {
            self.apply_reviewed_rename(dialog);
        }
    }

    fn request_rename_stage(&mut self, dialog: &RenameDialog) -> RenameStage {
        let name = dialog.draft.trim();
        if name.is_empty() {
            return RenameStage::Error("Enter a new symbol name.".into());
        }
        if self
            .native_docs
            .get(&dialog.path)
            .is_some_and(NativeDoc::dirty)
        {
            return RenameStage::Error("Save this file before renaming a symbol.".into());
        }
        match self.lsp.request_rename(&dialog.path, dialog.position, name) {
            Ok(()) => RenameStage::WaitingServer,
            Err(error) => RenameStage::Error(error),
        }
    }

    fn apply_reviewed_rename(&self, dialog: &mut RenameDialog) {
        let preview = std::mem::replace(&mut dialog.stage, RenameStage::Input);
        let RenameStage::Preview(prepared) = preview else {
            return;
        };
        let paths = prepared
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        dialog.stage = match self
            .rename_open_docs_are_clean(&prepared)
            .and_then(|()| self.start_rename_apply(prepared))
        {
            Ok(rx) => RenameStage::Applying { rx, paths },
            Err(error) => RenameStage::Error(error),
        };
    }
}
