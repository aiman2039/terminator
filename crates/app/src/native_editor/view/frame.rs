//! Per-frame values for one native view, read before the buffer borrow.
use super::*;

impl App {
    /// Everything the view reads from the app this frame, in the order
    /// the reads (and the requests some of them start) happen.
    pub(super) fn native_view_frame(
        &mut self,
        ui: &mut egui::Ui,
        site: ViewSite<'_>,
        services: gui_services::Services,
    ) -> (ViewFrame, SourceDecor) {
        let path = site.path;
        self.native_remote_banner(ui, path);
        let language: Language = language_for_path(&path.to_string_lossy());
        let is_markdown = markdown::supported(path);
        let preview = self.native_view_preview(ui, path);
        let source_id = self.native_view_source_id(ui, site);
        let rename_applying = self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(path));
        // Diagnostics snapshots for this frame: the manager borrow ends
        // before the buffer borrow begins.
        let marks: Vec<DiagnosticMark> = self.lsp.row_marks(path);
        let problems = self.lsp.editor_diagnostics(path);
        let problems_generation = self.lsp.diagnostics_generation();
        let reference = self
            .selected_project()
            .and_then(|project| path.strip_prefix(&project.path).ok())
            .unwrap_or(path)
            .display()
            .to_string();
        let annotations = self.native_view_annotations(path);
        let remote_snapshot = crate::remote_ui::is_snapshot(&self.paths.data, path);
        let blame_offer = (!remote_snapshot).then(|| self.editor_blame.contains_key(path));
        let inlay_answer = self
            .state
            .settings
            .inlay_hints
            .then(|| self.lsp.inlay_hints(path))
            .flatten();
        let git_base = if remote_snapshot {
            None
        } else {
            self.git_base(path)
        };
        let frame = ViewFrame {
            vim: self.state.settings.native_vim,
            failed_color: appearance::color(&self.theme.status_failed),
            services,
            language,
            is_markdown,
            preview,
            source_id,
            rename_applying,
            problems,
            problems_generation,
            blame_offer,
            inlay_answer,
            completion_triggers: self.lsp.completion_triggers(path),
            remote_snapshot,
            git_base,
            change_colors: [
                appearance::color(&self.theme.git_added),
                appearance::color(&self.theme.git_modified),
                appearance::color(&self.theme.git_deleted),
            ],
        };
        let decor = SourceDecor {
            marks,
            reference,
            annotations,
        };
        (frame, decor)
    }

    /// Read-only banner for a remote snapshot.
    fn native_remote_banner(&mut self, ui: &mut egui::Ui, path: &Path) {
        let remote_source = self.remote_file_source(path);
        if let Some((remote, relative)) = &remote_source {
            ui.horizontal(|ui| {
                ui.weak("Remote snapshot · read-only");
                if ui.button("Open in SSH editor").clicked() {
                    self.open_remote_file(OpenRemoteFileInput {
                        project: remote,
                        path: relative.clone(),
                        line: None,
                        split: None,
                        text: true,
                    });
                }
            });
        }
    }

    fn native_view_preview(&self, ui: &egui::Ui, path: &Path) -> ViewPreview {
        let key = format!("native:{}", path.display());
        let hovered = self
            .markdown
            .entries
            .get(&key)
            .is_some_and(|preview| preview.contains_pointer(ui));
        let scroll = if hovered {
            self.markdown
                .entries
                .get(&key)
                .map(|preview| preview.scroll_fraction)
        } else {
            None
        };
        ViewPreview {
            key,
            hovered,
            scroll,
        }
    }

    /// This pane's source widget id; takes a pending focus request aimed
    /// at this view once its buffer has loaded.
    fn native_view_source_id(&mut self, ui: &egui::Ui, site: ViewSite<'_>) -> String {
        let ViewSite {
            path,
            project,
            tab,
            node,
        } = site;
        let source_id = format!(
            "{:?}",
            ui.make_persistent_id((path, project, tab, node, ui.ctx().viewport_id()))
        );
        let focus_here = self.native_focus_pending.as_ref().is_some_and(|target| {
            project == Some(target.project.as_str())
                && tab == Some(target.group.as_str())
                && node == Some(target.path.node_path())
        }) && self
            .native_docs
            .get(path)
            .is_some_and(|doc| doc.doc.is_some());
        if focus_here {
            self.native_focus_pending = None;
            ui.memory_mut(|memory| memory.request_focus(source_focus_id(&source_id)));
        }
        source_id
    }

    /// Blame column for this frame (the last answer, moved onto edited
    /// lines, while a fresh blame runs).
    fn native_view_annotations(&mut self, path: &Path) -> Option<Arc<Vec<Annotation>>> {
        let buffer = self.native_docs.get(path)?.doc.as_ref()?;
        Some(self.editor_blame.get_mut(path)?.column(buffer))
    }

    /// Take this file's popups out of the app for the buffer borrow.
    pub(super) fn native_view_checkout(&mut self, path: &Path) -> ViewPopups {
        ViewPopups {
            find: self.find_bar.take_if(|bar| bar.path == path),
            completion: self.completion.take_if(|popup| popup.path == path),
            signature: self.signature.take_if(|popup| popup.path == path),
            actions: self.code_actions.take_if(|popup| popup.path == path),
            gutter: self.gutter_popup.take_if(|popup| popup.path == path),
        }
    }
}
