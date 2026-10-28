//! One Explorer folder or file row with its actions.
use super::*;

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
        appearance::context_menu(&folder, |ui| self.explorer_folder_menu(ui, &folder_path));
    }

    /// A right click on an unselected row selects just that row.
    fn explorer_select_on_secondary(&mut self, response: &egui::Response, path: &Path) {
        if response.secondary_clicked() && !self.explorer_selected(path, None) {
            self.explorer_selection = vec![path.to_owned()];
            self.explorer_anchor = Some(path.to_owned());
        }
    }

    fn explorer_folder_menu(&mut self, ui: &mut egui::Ui, folder_path: &Path) {
        self.explorer_folder_menu_create(ui, folder_path);
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Rename",
            icon: "Pencil",
            shortcut: "",
        })
        .clicked()
        {
            self.explorer_rename(folder_path);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Duplicate",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            self.queue_workspace(workspace_ops::Op::Duplicate(folder_path.to_owned()));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Delete",
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.explorer_confirm_delete();
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Collapse all",
            icon: "ChevronDown",
            shortcut: "",
        })
        .clicked()
        {
            self.expanded_dirs.clear();
            ui.close();
        }
    }

    /// New file/folder, Reveal, Copy path, and Paste.
    fn explorer_folder_menu_create(&mut self, ui: &mut egui::Ui, folder_path: &Path) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New file",
            icon: "File",
            shortcut: "",
        })
        .clicked()
        {
            self.open_name_prompt(workspace_ops::NamePrompt::File {
                dir: folder_path.to_owned(),
                name: String::new(),
            });
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New folder",
            icon: "Folder",
            shortcut: "",
        })
        .clicked()
        {
            self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                dir: folder_path.to_owned(),
                name: String::new(),
            });
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Reveal in file manager",
            icon: "FolderOpen",
            shortcut: "",
        })
        .clicked()
        {
            self.queue_workspace(workspace_ops::Op::Reveal(folder_path.to_owned()));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Copy path",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            ui.ctx().copy_text(folder_path.display().to_string());
            ui.close();
        }
        if self.explorer_clipboard.is_some()
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Paste",
                icon: "ClipboardPaste",
                shortcut: "",
            })
            .clicked()
        {
            self.explorer_selection = vec![folder_path.to_owned()];
            let root = folder_path.to_owned();
            self.explorer_paste(&root);
            ui.close();
        }
    }

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
