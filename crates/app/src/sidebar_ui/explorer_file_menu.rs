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
    pub(super) perforce: bool,
    /// The file is in a Git repository.
    pub(super) git: bool,
    pub(super) marks: FileRowMarks,
}

impl FileRowMenu<'_> {
    pub(super) fn show(&self, ui: &mut egui::Ui, outcome: &mut ExplorerRowOutcome) {
        if let Some(action) = file_actions::menu(
            ui,
            file_actions::FileMenu {
                file: true,
                browser: file_actions::browser_document(self.path),
                git: None,
                neovim: false,
                url: false,
            },
        ) {
            outcome.menu = Some(action);
        }
        if self.perforce {
            ui.separator();
            self.perforce_items(ui, outcome);
        }
        if self.git {
            ui.separator();
            Self::git_items(ui, outcome);
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

    /// Show history, Compare with…, and Compare with Revision….
    fn git_items(ui: &mut egui::Ui, outcome: &mut ExplorerRowOutcome) {
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
        for (label, icon, mode) in crate::revision_compare::MENU_ITEMS {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: "",
            })
            .clicked()
            {
                outcome.local = Some(ExplorerLocal::Compare(mode));
                ui.close();
            }
        }
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
        if marks.local_history {
            item(ui, "Local History", "History", ExplorerLocal::LocalHistory);
        }
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

#[cfg(test)]
mod tests {
    use super::super::explorer_rows::{ExplorerFileRowInput, explorer_file_row};
    use super::*;
    use terminator_core::appearance::AppearanceConfig;

    /// One frame of an Explorer file row and its menu; returns painted texts.
    fn row_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let theme = AppearanceConfig::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                explorer_file_row(
                    ui,
                    ExplorerFileRowInput {
                        path: Path::new("/a/main.rs"),
                        label: "main.rs",
                        status: ' ',
                        ignored: false,
                        errors: false,
                        theme: &theme,
                        selected: false,
                        perforce: false,
                        git: false,
                        marks: FileRowMarks::default(),
                    },
                );
            },
        );
        output.textures_delta.clear();
        crate::app::painted_text(&output.shapes)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    #[test]
    fn explorer_file_menu_open_rows_show_no_file_picker_or_split_keys() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let pos = egui::pos2(40.0, 12.0);
        row_frame(&ctx, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            let click = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            row_frame(&ctx, vec![click]);
        }
        // The menu's first frame is a hidden sizing pass.
        row_frame(&ctx, vec![]);
        let texts = row_frame(&ctx, vec![]);
        for label in ["Open file", "Open in editor split"] {
            assert!(texts.iter().any(|text| text == label), "{label}: {texts:?}");
        }
        let keys = terminator_core::Settings::default().keybindings;
        for action in ["open_file", "split_right"] {
            let hint = crate::shortcuts::pretty(&keys, action);
            assert!(!hint.is_empty(), "{action} has a default key");
            assert!(!texts.contains(&hint), "{action} {hint:?}: {texts:?}");
        }
    }
}
