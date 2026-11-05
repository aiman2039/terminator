//! Explorer status rows: a folder that is loading, missing, or unreadable.
use super::*;
use crate::app::missing_folder::{folder_missing, missing_folder_text};

/// macOS access hint for folders that exist but cannot be read.
const ACCESS_HINT: &str = "Check System Settings → Privacy & Security → Files and Folders. Full Disk Access is optional troubleshooting; this error may have another cause. Shells and editors can have separate access.";

impl App {
    /// `root` marks the Explorer root row: a missing root offers the
    /// project actions.
    pub(super) fn explorer_directory_status(&mut self, ui: &mut egui::Ui, path: &Path, root: bool) {
        if let Some(error) = self.directory_errors.get(path).cloned() {
            if folder_missing(error.kind) {
                self.explorer_missing_folder(ui, path, root);
            } else {
                self.explorer_unreadable_folder(ui, path, &error);
            }
        }
        if !self.dirs.contains_key(path) && !self.directory_errors.contains_key(path) {
            ui.weak("Loading…");
        }
    }

    /// A moved or deleted folder. No access hint: access is not the cause.
    /// A deleted terminal folder is not a root here: the Explorer shows the
    /// project folder in its place.
    fn explorer_missing_folder(&mut self, ui: &mut egui::Ui, path: &Path, root: bool) {
        ui.colored_label(ui.visuals().error_fg_color, missing_folder_text(path));
        self.explorer_last_listing(ui, path);
        let project = if root {
            self.missing_root_project(path)
        } else {
            None
        };
        ui.horizontal_wrapped(|ui| {
            if let Some(project) = &project {
                let locate = ui.button("Locate Folder…");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "directory-locate", locate.rect);
                if locate.clicked() {
                    self.locate_missing_folder(project);
                }
                let remove = ui.button("Remove Project");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "directory-remove-project", remove.rect);
                if remove.clicked() {
                    self.hide_project(project);
                }
            }
            self.explorer_retry(ui);
        });
    }

    /// A folder that exists but cannot be read, for example without access.
    fn explorer_unreadable_folder(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        error: &crate::services::DirectoryError,
    ) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Cannot refresh {}: {}", error.path.display(), error.message),
        );
        self.explorer_last_listing(ui, path);
        ui.horizontal(|ui| {
            self.explorer_retry(ui);
            if ui.button("Choose folder again").clicked() {
                self.add_project = true;
            }
        });
        if cfg!(target_os = "macos") {
            ui.weak(ACCESS_HINT);
        }
    }

    fn explorer_last_listing(&self, ui: &mut egui::Ui, path: &Path) {
        if self.dirs.contains_key(path) {
            ui.weak("Showing the last successful listing.");
        }
    }

    fn explorer_retry(&mut self, ui: &mut egui::Ui) {
        let retry = ui.button("Retry");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "directory-retry", retry.rect);
        if retry.clicked() {
            self.retry_explorer();
        }
    }
}
