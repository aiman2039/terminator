//! Project list drag and drop: drag a project row onto another to reorder
//! the list (Manual sort), and drop folders from the OS file manager to
//! add them as projects.
use super::*;
use std::path::PathBuf;

/// Drag payload: the id of the project row being dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectDrag(String);

/// The note shown after folders are dropped on the project list: one line
/// per folder, so each drop has a visible result.
fn dropped_projects_note(paths: &[PathBuf]) -> String {
    let mut note = match paths.len() {
        1 => "Adding project:".to_owned(),
        count => format!("Adding {count} projects:"),
    };
    for path in paths {
        note.push('\n');
        note.push_str(&path.display().to_string());
    }
    note.push_str("\nAn item that is not a folder shows an error.");
    note
}

impl App {
    /// A project row is a drag source and a drop target for other rows.
    /// A drop moves the dragged project to the target's place and turns
    /// on Manual sort.
    pub(super) fn project_row_dnd(&mut self, ui: &egui::Ui, response: &egui::Response, id: &str) {
        let drag = response.interact(egui::Sense::drag());
        if drag.dragged() {
            drag.dnd_set_drag_payload(ProjectDrag(id.to_owned()));
        }
        if response
            .dnd_hover_payload::<ProjectDrag>()
            .is_some_and(|payload| payload.0 != id)
        {
            ui.painter().rect_stroke(
                response.rect,
                appearance::WIDGET_RADIUS,
                egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<ProjectDrag>() {
            let shown: Vec<String> = self
                .cached_projects()
                .iter()
                .map(|p| p.id.clone())
                .collect();
            crate::preferences::reorder_projects(
                &mut self.preferences.project_order,
                &shown,
                &payload.0,
                id,
            );
            self.preferences.project_sort = ProjectSort::Manual;
        }
    }

    /// Folders dropped from the OS file manager on the project list (`rect`)
    /// open as projects, through the same job as New project.
    pub(super) fn project_folder_drop(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        // Only with the pointer known to be here: the Explorer takes drops
        // when the position is unknown.
        let inside = ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|pointer| rect.contains(pointer));
        if !inside {
            return;
        }
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                rect,
                appearance::RADIUS_SMALL,
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
        if dropped.is_empty() {
            return;
        }
        // `OpenProject` checks for a folder and sends `AddProject` off the
        // GUI thread; a failure reports its own error.
        let generation = self.selection_generation;
        for path in &dropped {
            let _ = self
                .jobs
                .send(crate::Job::OpenProject(path.clone(), generation));
        }
        self.info = Some(dropped_projects_note(&dropped));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_folders_are_each_named_in_the_note() {
        let note = dropped_projects_note(&["/a".into(), "/b".into()]);
        assert!(note.starts_with("Adding 2 projects:"));
        assert!(note.contains("\n/a\n/b\n"));
        assert!(dropped_projects_note(&["/a".into()]).starts_with("Adding project:\n/a"));
    }
}
