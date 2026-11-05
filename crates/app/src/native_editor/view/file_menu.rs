//! A native editor's file actions in its pane tab menu: reload from disk,
//! the file bookmark, recent files, and Local History.
use super::*;

impl App {
    /// File actions for the native editor showing `path`.
    pub(crate) fn native_file_menu(&mut self, ui: &mut egui::Ui, path: &Path) {
        self.native_reload_item(ui, path);
        let pinned = self.preferences.pinned_native_files.contains(path);
        if menu_click(
            ui,
            ("Bookmark file", "Bookmark", if pinned { "✓" } else { "" }),
        ) {
            if pinned {
                self.preferences.pinned_native_files.remove(path);
            } else {
                self.preferences.pinned_native_files.insert(path.to_owned());
            }
        }
        self.native_recent_menu(ui, path);
        let local = !crate::remote_ui::is_snapshot(&self.paths.data, path);
        if local
            && self.local_history_supported()
            && menu_click(ui, ("Local History", "History", ""))
        {
            self.open_local_history(path.to_owned());
        }
    }

    /// Reload while dirty would silently discard edits: disabled.
    fn native_reload_item(&mut self, ui: &mut egui::Ui, path: &Path) {
        let applying = self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(path));
        let doc = self
            .native_docs
            .get_mut(path)
            .filter(|doc| doc.doc.is_some() && !doc.dirty() && !doc.loading && !applying);
        let reload = ui
            .add_enabled_ui(doc.is_some(), |ui| {
                appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label: "Reload from disk",
                    icon: "RefreshCw",
                    shortcut: "",
                })
            })
            .inner
            .on_disabled_hover_text("Unsaved changes are kept");
        if reload.clicked()
            && let Some(doc) = doc
        {
            // Applied as one undoable edit: undo history survives a reload.
            doc.request_reload(false);
            ui.close();
        }
    }

    /// Recent native files of the selected project, bookmarked ones first.
    fn native_recent_menu(&mut self, ui: &mut egui::Ui, path: &Path) {
        let recent = self.native_recent_files(path);
        let pinned = &self.preferences.pinned_native_files;
        let picked =
            appearance::menu_button(ui, "Recent files", |ui| recent_items(ui, &recent, pinned))
                .inner
                .flatten();
        if let Some(target) = picked {
            // No target line: the file opens where it was left.
            self.queue_jump(path, &target, None);
        }
    }

    /// The selected project's recent native files except `path`, bookmarked
    /// ones first.
    pub(super) fn native_recent_files(&self, path: &Path) -> Vec<PathBuf> {
        let Some(root) = self.selected_project().map(|project| &project.path) else {
            return Vec::new();
        };
        let pinned = &self.preferences.pinned_native_files;
        let mut recent: Vec<PathBuf> = self
            .preferences
            .recent_native_files
            .iter()
            .filter(|item| item.starts_with(root) && item.as_path() != path)
            .cloned()
            .collect();
        recent.sort_by_key(|item| !pinned.contains(item));
        recent
    }
}

/// One menu row; a click closes the menu.
fn menu_click(ui: &mut egui::Ui, (label, icon, shortcut): (&str, &str, &str)) -> bool {
    let clicked = appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut,
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// The recent-files submenu; returns the clicked file.
fn recent_items(
    ui: &mut egui::Ui,
    recent: &[PathBuf],
    pinned: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    if recent.is_empty() {
        ui.weak("No other recent files in this project");
    }
    let mut picked = None;
    for item in recent {
        let name = item.file_name().unwrap_or_default().to_string_lossy();
        let shortcut = if pinned.contains(item) {
            "Bookmarked"
        } else {
            ""
        };
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: &name,
            icon: "FileText",
            shortcut,
        })
        .on_hover_text(item.display().to_string())
        .clicked()
        {
            picked = Some(item.clone());
            ui.close();
        }
    }
    picked
}
