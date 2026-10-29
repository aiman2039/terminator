//! One Explorer folder or file row with its actions.
use super::*;
use crate::sidebar_ui::explorer_rows::ExplorerLocal;

impl App {
    pub(super) fn explorer_entry(&mut self, input: ExplorerEntryInput<'_, '_>) {
        if input.entry.directory {
            self.explorer_folder_entry(input);
        } else {
            self.explorer_file_entry(input);
        }
    }

    /// Git decoration letter for `path` (space when clean).
    fn explorer_status(&self, path: &Path) -> char {
        self.context
            .as_ref()
            .and_then(|c| c.decorations.get(path))
            .copied()
            .unwrap_or(' ')
    }

    fn explorer_folder_entry(&mut self, input: ExplorerEntryInput<'_, '_>) {
        let ExplorerEntryInput {
            ui,
            entry,
            row,
            context,
            drop_dir,
        } = input;
        let label = row.label.as_str();
        let modifiers = ui.input(|input| input.modifiers);
        let expanded = self.explorer_row_expanded(row);
        let status = self.explorer_status(&entry.path);
        let color = if entry.ignored {
            appearance::color(&self.theme.git_ignored)
        } else {
            git_color(&self.theme, status)
        };
        let folder = appearance::file_row(appearance::FileRowInput {
            ui,
            label,
            icon: if expanded { "FolderOpen" } else { "Folder" },
            selected: self.explorer_selected(&entry.path, None),
            height: 22.0,
            trailing: &status.to_string(),
            tint: color,
        })
        .interact(egui::Sense::drag());
        self.explorer_row_dnd(ExplorerRowDndInput {
            ui,
            response: &folder,
            path: &entry.path,
            directory: true,
            drop_dir,
        });
        // Build the tooltip lazily: `format!` per visible row per
        // frame showed up as scroll-time allocations.
        let folder = if folder.hovered() {
            folder.on_hover_text(format!(
                "{}\n{}",
                entry.path.display(),
                if entry.ignored {
                    "Ignored"
                } else {
                    terminator_git::status_description(status)
                }
            ))
        } else {
            folder
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("explorer-folder:{label}"), folder.rect);
        if folder.clicked() {
            ui.memory_mut(|memory| memory.request_focus(context.focus));
            if self.explorer_click(context.rows, &entry.path, modifiers) {
                self.set_explorer_row_expanded(row, !expanded);
            }
        }
        self.explorer_select_on_secondary(&folder, &entry.path);
        let folder_path = entry.path.clone();
        // A compact row (`a/b/c`) collapses from its top folder.
        let top = row.chain.first().unwrap_or(&entry.path).clone();
        appearance::context_menu(&folder, |ui| {
            self.explorer_folder_menu(ui, &folder_path, &top);
        });
    }

    /// A right click on an unselected row selects just that row.
    fn explorer_select_on_secondary(&mut self, response: &egui::Response, path: &Path) {
        if response.secondary_clicked() && !self.explorer_selected(path, None) {
            self.explorer_selection = vec![path.to_owned()];
            self.explorer_anchor = Some(path.to_owned());
        }
    }

    fn explorer_folder_menu(&mut self, ui: &mut egui::Ui, folder_path: &Path, top: &Path) {
        if menu_click(ui, "New file", "File") {
            self.open_name_prompt(workspace_ops::NamePrompt::File {
                dir: folder_path.to_owned(),
                name: String::new(),
            });
        }
        if menu_click(ui, "New folder", "Folder") {
            self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                dir: folder_path.to_owned(),
                name: String::new(),
            });
        }
        if menu_click(ui, "Open in terminal", "Terminal") {
            self.create_workspace_tab_in(None, Some(folder_path.to_owned()));
        }
        ui.separator();
        self.explorer_folder_menu_clipboard(ui, folder_path);
        ui.separator();
        if menu_click(ui, "Find in folder", "Search") {
            self.explorer_find_in_folder(folder_path);
        }
        if menu_click(ui, "Reveal in file manager", "FolderOpen") {
            self.queue_workspace(workspace_ops::Op::Reveal(folder_path.to_owned()));
        }
        ui.separator();
        if menu_click(ui, "Rename", "Pencil") {
            self.explorer_rename(folder_path);
        }
        if menu_click(ui, "Duplicate", "Copy") {
            self.queue_workspace(workspace_ops::Op::Duplicate(folder_path.to_owned()));
        }
        if menu_click(ui, "Delete", "X") {
            self.explorer_confirm_delete();
        }
        ui.separator();
        if menu_click(ui, "Collapse all", "ChevronDown") {
            self.expanded_dirs.retain(|dir| !dir.starts_with(top));
        }
    }

    /// Cut, Copy, Paste, and the copy-path variants, as for files.
    fn explorer_folder_menu_clipboard(&mut self, ui: &mut egui::Ui, folder_path: &Path) {
        if menu_click(ui, "Cut", "Scissors") {
            self.explorer_copy_selection(ui, true);
        }
        if menu_click(ui, "Copy", "Copy") {
            self.explorer_copy_selection(ui, false);
        }
        if self.explorer_clipboard.is_some() && menu_click(ui, "Paste", "ClipboardPaste") {
            self.explorer_selection = vec![folder_path.to_owned()];
            self.explorer_paste(folder_path);
        }
        if menu_click(ui, "Copy path", "Copy") {
            ui.ctx().copy_text(folder_path.display().to_string());
        }
        if menu_click(ui, "Copy relative path", "Copy") {
            self.explorer_local(folder_path, ExplorerLocal::CopyRelative, ui);
        }
        if menu_click(ui, "Copy file name", "Copy") {
            self.explorer_local(folder_path, ExplorerLocal::CopyName, ui);
        }
    }

    /// Contents search limited to `folder` (JetBrains Find in Path with a
    /// directory scope).
    fn explorer_find_in_folder(&mut self, folder: &Path) {
        let Some(cwd) = self.cwd() else {
            return;
        };
        self.preferences.explorer_search_mode = ExplorerSearchMode::Contents;
        self.preferences.explorer_include = crate::search::folder_include_pattern(&cwd, folder);
        self.explorer_focus_find = true;
    }
}

/// One menu entry; closes the menu when picked.
fn menu_click(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut: "",
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl App {
    fn explorer_file_entry(&mut self, input: ExplorerEntryInput<'_, '_>) {
        let ExplorerEntryInput {
            ui,
            entry,
            row,
            context,
            drop_dir,
        } = input;
        let modifiers = ui.input(|input| input.modifiers);
        let status = self.explorer_status(&entry.path);
        let outcome = explorer_file_row(
            ui,
            ExplorerFileRowInput {
                path: &entry.path,
                label: row.label.as_str(),
                status,
                ignored: entry.ignored,
                theme: &self.theme,
                open_shortcut: &context.open_shortcut,
                split_shortcut: &context.split_shortcut,
                selected: self.explorer_selected(&entry.path, context.focused_file.as_deref()),
                perforce: self
                    .context
                    .as_ref()
                    .is_some_and(|context| context.perforce.is_some()),
                git_branches: self
                    .context
                    .as_ref()
                    .and_then(|context| context.root.as_ref())
                    .map(|_| self.git.branches.as_slice()),
            },
        );
        if let Some(response) = &outcome.response {
            self.explorer_row_dnd(ExplorerRowDndInput {
                ui,
                response,
                path: &entry.path,
                directory: false,
                drop_dir,
            });
            self.explorer_select_on_secondary(response, &entry.path);
        }
        if let Some(action) = outcome.clicked {
            ui.memory_mut(|memory| memory.request_focus(context.focus));
            if self.explorer_click(context.rows, &entry.path, modifiers) {
                self.activate_file_action(ui, &entry.path, action);
            }
        }
        if let Some(action) = outcome.menu {
            self.file_action(FileActionInput {
                ui,
                action,
                path: &entry.path,
                line: None,
            });
        }
        if let Some(op) = outcome.perforce {
            self.queue_workspace(workspace_ops::Op::Perforce(op));
        }
        if let Some(local) = outcome.local {
            self.explorer_local(&entry.path, local, ui);
        }
    }
}
