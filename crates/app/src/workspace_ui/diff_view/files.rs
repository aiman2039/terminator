//! Changed-files panel inside the diff: every file on this diff's side,
//! grouped by folder. A click opens that file's diff in the same pane.
use super::super::super::*;
use super::{DiffFrame, DiffPass};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One changed file on the diff's side, with its Git status letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChangedFile {
    pub(super) path: PathBuf,
    pub(super) letter: char,
}

/// Row height of the changed-files panel.
const FILE_ROW_HEIGHT: f32 = 22.0;

/// Files grouped by their folder relative to `root`, folders in name
/// order and files in Git's order inside each folder.
pub(super) fn group_by_folder<'a>(
    files: &'a [ChangedFile],
    root: &Path,
) -> BTreeMap<String, Vec<&'a ChangedFile>> {
    let mut groups: BTreeMap<String, Vec<&ChangedFile>> = BTreeMap::new();
    for file in files {
        let relative = file.path.strip_prefix(root).unwrap_or(&file.path);
        let folder = relative
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();
        groups.entry(folder).or_default().push(file);
    }
    groups
}

/// Sidebar Git colors by status letter, so the panel matches the sidebar.
fn status_color(theme: &terminator_core::appearance::AppearanceConfig, letter: char) -> Color32 {
    appearance::color(match letter {
        'A' => &theme.git_added,
        'M' => &theme.git_modified,
        'D' | '!' => &theme.git_deleted,
        'R' | 'C' | 'U' => &theme.git_untracked,
        _ => &theme.secondary,
    })
}

impl App {
    /// Changed files on the staged or working-tree side, in Git's order.
    pub(super) fn diff_changed_files(&self, staged_side: bool) -> Vec<ChangedFile> {
        use terminator_git::GitGroup;
        let Some(context) = self.context.as_ref() else {
            return Vec::new();
        };
        context
            .changes
            .iter()
            .filter_map(|change| {
                let group = if staged_side {
                    change
                        .in_group(GitGroup::Staged)
                        .then_some(GitGroup::Staged)
                } else if change.in_group(GitGroup::Changes) {
                    Some(GitGroup::Changes)
                } else {
                    change
                        .in_group(GitGroup::Untracked)
                        .then_some(GitGroup::Untracked)
                }?;
                Some(ChangedFile {
                    path: change.path.clone(),
                    letter: change.letter(group),
                })
            })
            .collect()
    }

    /// The panel itself: folder headers and clickable file rows.
    pub(super) fn diff_file_tree(
        &self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        let files = self.diff_changed_files(frame.staged && !frame.read_only);
        let current = if frame.path.is_absolute() {
            frame.path.to_path_buf()
        } else {
            frame.cwd.join(frame.path)
        };
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.strong("Changed files");
            ui.weak(files.len().to_string());
        });
        if files.is_empty() {
            ui.weak("No changed files on this side");
            return;
        }
        let root = self
            .context
            .as_ref()
            .and_then(|context| context.root.clone())
            .unwrap_or_else(|| frame.cwd.to_path_buf());
        egui::ScrollArea::vertical()
            .id_salt(("diff-file-tree", &frame.key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let weak = ui.visuals().weak_text_color();
                for (folder, entries) in group_by_folder(&files, &root) {
                    if !folder.is_empty() {
                        appearance::file_row(appearance::FileRowInput {
                            ui,
                            label: &folder,
                            icon: "Folder",
                            selected: false,
                            height: FILE_ROW_HEIGHT,
                            trailing: "",
                            tint: weak,
                        });
                    }
                    for file in entries {
                        self.diff_file_row(ui, (file, &current), pass);
                    }
                }
            });
    }

    fn diff_file_row(
        &self,
        ui: &mut egui::Ui,
        (file, current): (&ChangedFile, &Path),
        pass: &mut DiffPass,
    ) {
        let name = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let selected = file.path.components().eq(current.components());
        let response = ui
            .indent(("diff-file", &file.path), |ui| {
                appearance::file_row(appearance::FileRowInput {
                    ui,
                    label: &name,
                    icon: crate::icons::file_icon(&file.path),
                    selected,
                    height: FILE_ROW_HEIGHT,
                    trailing: &file.letter.to_string(),
                    tint: status_color(&self.theme, file.letter),
                })
            })
            .inner
            .on_hover_text(format!(
                "{}\n{}",
                file.path.display(),
                terminator_git::status_description(file.letter)
            ));
        if response.clicked() && !selected {
            pass.navigate = Some(file.path.clone());
        }
    }
}
