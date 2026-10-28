//! Explorer drag-and-drop: row moves and drops from the OS file manager.
use super::*;

impl App {
    /// Files dropped from the OS file manager onto the tree copy into the
    /// folder under the pointer (or the tree root).
    pub(super) fn explorer_external_drop(
        &mut self,
        ui: &egui::Ui,
        root: &Path,
        drop_dir: Option<PathBuf>,
    ) {
        let clip = ui.clip_rect();
        let pointer_inside = ui
            .ctx()
            .pointer_latest_pos()
            .is_none_or(|pointer| clip.contains(pointer));
        if !pointer_inside {
            return;
        }
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                clip,
                4.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
        }
        let dropped: Vec<PathBuf> = ui.ctx().input_mut(|input| {
            std::mem::take(&mut input.raw.dropped_files)
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.queue_workspace(workspace_ops::Op::Copy {
                from: dropped,
                to_dir: drop_dir.unwrap_or_else(|| root.to_owned()),
            });
        }
    }

    /// Drag source and drop target wiring shared by file and folder rows.
    /// Dropping on a folder moves into it; on a file, into its folder.
    pub(super) fn explorer_row_dnd(&mut self, input: ExplorerRowDndInput<'_>) {
        let ExplorerRowDndInput {
            ui,
            response,
            path,
            directory,
            drop_dir,
        } = input;
        let target = if directory {
            Some(path.to_owned())
        } else {
            path.parent().map(Path::to_path_buf)
        };
        if response.contains_pointer() {
            drop_dir.clone_from(&target);
        }
        if response.dragged() {
            let paths = if self.explorer_selection.iter().any(|p| p == path) {
                self.explorer_selection.clone()
            } else {
                vec![path.to_owned()]
            };
            response.dnd_set_drag_payload(ExplorerDrag(paths));
        }
        if directory && response.dnd_hover_payload::<ExplorerDrag>().is_some() {
            ui.painter().rect_stroke(
                response.rect,
                appearance::WIDGET_RADIUS,
                egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<ExplorerDrag>()
            && let Some(target) = target
        {
            self.explorer_drop(payload.0.clone(), target);
        }
    }
}
