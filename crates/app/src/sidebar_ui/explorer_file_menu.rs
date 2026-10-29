//! The Explorer file row's context menu: open actions, Perforce and Git
//! items, and local file actions.
use super::explorer_rows::{ExplorerLocal, ExplorerRowOutcome, FileRowMarks};
use crate::{
    appearance,
    file_actions::{self, FileAction},
};
use eframe::egui;
use std::path::Path;

pub(super) struct FileRowMenu<'a> {
    pub(super) path: &'a Path,
    pub(super) open_shortcut: &'a str,
    pub(super) split_shortcut: &'a str,
    pub(super) perforce: bool,
    pub(super) git_branches: Option<&'a [String]>,
    pub(super) marks: FileRowMarks,
}

impl FileRowMenu<'_> {
    pub(super) fn show(&self, ui: &mut egui::Ui, outcome: &mut ExplorerRowOutcome) {
        if let Some(action) = file_actions::menu_with(file_actions::MenuWithInput {
            ui,
            spec: file_actions::FileMenu {
                file: true,
                browser: file_actions::browser_document(self.path),
                git: None,
                neovim: false,
            },
            open: self.open_shortcut,
            split: self.split_shortcut,
        }) {
            outcome.menu = Some(action);
        }
        if self.perforce {
            ui.separator();
            self.perforce_items(ui, outcome);
        }
        if let Some(branches) = self.git_branches {
            ui.separator();
            Self::git_items(ui, branches, outcome);
        }
        ui.separator();
        Self::local_items(ui, outcome, self.marks);
    }

    fn perforce_items(&self, ui: &mut egui::Ui, outcome: &mut ExplorerRowOutcome) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Perforce: Open for edit",
            icon: "Pencil",
            shortcut: "",
        })
        .clicked()
        {
            outcome.perforce = Some(crate::perforce::Op::Edit(self.path.into()));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Perforce: Reconcile file",
            icon: "RefreshCw",
            shortcut: "",
        })
        .clicked()
        {
            outcome.perforce = Some(crate::perforce::Op::Reconcile(self.path.into()));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Perforce: Diff",
            icon: "FileDiff",
            shortcut: "",
        })
        .clicked()
        {
            outcome.menu = Some(FileAction::NativeWorkingDiff);
            ui.close();
        }
    }

    fn git_items(ui: &mut egui::Ui, branches: &[String], outcome: &mut ExplorerRowOutcome) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Show history",
            icon: "History",
            shortcut: "",
        })
        .clicked()
        {
            outcome.local = Some(ExplorerLocal::History);
            ui.close();
        }
        ui.menu_button("Compare with branch", |ui| {
            if branches.is_empty() {
                ui.weak("Loading branches…");
            }
            for branch in branches {
                if ui.button(branch).clicked() {
                    outcome.local = Some(ExplorerLocal::CompareWith(branch.clone()));
                    ui.close();
                }
            }
        });
    }

    /// Reveal, copy, rename, duplicate, and delete.
    fn local_items(ui: &mut egui::Ui, outcome: &mut ExplorerRowOutcome, marks: FileRowMarks) {
        let mut item = |ui: &mut egui::Ui, label: &str, icon: &str, local: ExplorerLocal| {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: "",
            })
            .clicked()
            {
                outcome.local = Some(local);
                ui.close();
            }
        };
        item(
            ui,
            "Reveal in file manager",
            "FolderOpen",
            ExplorerLocal::Reveal,
        );
        item(
            ui,
            "Copy relative path",
            "Copy",
            ExplorerLocal::CopyRelative,
        );
        item(ui, "Copy file name", "Copy", ExplorerLocal::CopyName);
        ui.separator();
        item(ui, "Rename", "Pencil", ExplorerLocal::Rename);
        item(ui, "Duplicate", "Copy", ExplorerLocal::Duplicate);
        ui.separator();
        item(ui, "Delete", "X", ExplorerLocal::Delete);
        ui.separator();
        if let Some(open) = marks.nested {
            let label = if open {
                "Hide nested files"
            } else {
                "Show nested files"
            };
            item(ui, label, "ChevronDown", ExplorerLocal::ToggleNested);
        }
        let bookmark = if marks.bookmarked {
            "Remove bookmark"
        } else {
            "Add bookmark"
        };
        item(ui, bookmark, "Bookmark", ExplorerLocal::ToggleBookmark);
        let exclude = if marks.excluded {
            "Show in Explorer"
        } else {
            "Hide from Explorer"
        };
        item(ui, exclude, "EyeOff", ExplorerLocal::ToggleExcluded);
    }
}
