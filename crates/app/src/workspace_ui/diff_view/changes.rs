//! All changes: every changed file of one side in one scrolling view. Each
//! open section reuses the single-file diff (hunk actions, commit lock,
//! staging) and loads only while it is on screen.
use super::super::super::*;
use super::files::ChangedFile;
use std::path::Path;

/// Sections open at first; later files start collapsed.
const EAGER_FILES: usize = 50;
/// Approximate height of one diff row, for sizing a section.
const ROW_HEIGHT: f32 = 18.0;
const MIN_SECTION: f32 = 160.0;
const MAX_SECTION: f32 = 560.0;

/// Height of a file section from its changed line count, with room for the
/// toolbar and some context.
pub(super) fn section_height(added: u32, deleted: u32) -> f32 {
    let rows = added.saturating_add(deleted).saturating_add(8);
    (f32::from(u16::try_from(rows).unwrap_or(u16::MAX)) * ROW_HEIGHT)
        .clamp(MIN_SECTION, MAX_SECTION)
}

/// One file section: its file, the added/deleted line counts, and whether
/// it starts open.
struct Section {
    file: ChangedFile,
    added: u32,
    deleted: u32,
    open: bool,
}

impl App {
    pub(in super::super) fn changes_diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::ChangesDiff { cwd, staged } = tab else {
            return;
        };
        let root = self
            .context
            .as_ref()
            .and_then(|context| context.root.as_deref());
        if root != Some(cwd.as_path()) {
            ui.weak("Select a terminal or file in this repository to list its changes.");
            return;
        }
        let sections = self.changes_sections(*staged);
        let (added, deleted) = sections.iter().fold((0u32, 0u32), |(a, d), s| {
            (a.saturating_add(s.added), d.saturating_add(s.deleted))
        });
        ui.horizontal(|ui| {
            ui.strong(tab.git_title());
            ui.weak(format!("{} files", sections.len()));
            self.line_counts(ui, added, deleted);
        });
        if sections.is_empty() {
            ui.weak("No changes.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("changes-diff", tab.key()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for section in &sections {
                    self.changes_section(ui, (cwd, *staged), section);
                }
            });
    }

    fn changes_sections(&self, staged: bool) -> Vec<Section> {
        let stats = self.context.as_ref().map(|context| &context.stats);
        self.diff_changed_files(staged)
            .into_iter()
            .enumerate()
            .map(|(index, file)| {
                let (added, deleted) = stats
                    .and_then(|stats| stats.get(&file.path).copied())
                    .unwrap_or_default();
                Section {
                    file,
                    added,
                    deleted,
                    open: index < EAGER_FILES,
                }
            })
            .collect()
    }

    fn line_counts(&self, ui: &mut egui::Ui, added: u32, deleted: u32) {
        ui.colored_label(
            appearance::color(&self.theme.git_added),
            RichText::new(format!("+{added}")).small(),
        );
        ui.colored_label(
            appearance::color(&self.theme.git_deleted),
            RichText::new(format!("−{deleted}")).small(),
        );
    }

    /// A collapsible header (path and line counts) over the file's diff.
    fn changes_section(
        &mut self,
        ui: &mut egui::Ui,
        (root, staged): (&Path, bool),
        section: &Section,
    ) {
        let path = &section.file.path;
        let id = ui.make_persistent_id(("changes-file", path, staged));
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            section.open,
        );
        let label = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        state
            .show_header(ui, |ui| {
                ui.strong(label);
                self.line_counts(ui, section.added, section.deleted);
            })
            .body_unindented(|ui| {
                let height = section_height(section.added, section.deleted);
                let rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), height),
                );
                if ui.is_rect_visible(rect) {
                    let tab = Tab::Diff {
                        remote: None,
                        cwd: root.to_path_buf(),
                        path: path.clone(),
                        staged,
                    };
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                        self.embedded_diff(ui, &tab);
                    });
                }
                ui.allocate_rect(rect, egui::Sense::hover());
            });
    }

    /// The single-file diff without its changed-files panel: this view is
    /// already the file list.
    fn embedded_diff(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let tree = std::mem::replace(&mut self.preferences.diff_file_tree, false);
        self.diff_view(ui, tab);
        self.preferences.diff_file_tree = tree;
    }

    /// Opens (or focuses) the All changes view of the Git panel's repository.
    pub(crate) fn open_changes_diff(&mut self, staged: bool) {
        let (Some(project), Some(cwd)) = (self.selected.clone(), self.git_root()) else {
            self.info = Some("Select a Git project to show its changes.".into());
            return;
        };
        let tab = Tab::ChangesDiff { cwd, staged };
        if self.focus_file_view(&project, &tab) {
            return;
        }
        self.layouts
            .entry(project)
            .or_insert_with(Workspace::empty)
            .add(id(), tab);
        self.focus_non_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_height_grows_with_changes_within_bounds() {
        assert_eq!(section_height(0, 0), MIN_SECTION);
        assert!(section_height(10, 5) > MIN_SECTION);
        assert_eq!(section_height(5000, 0), MAX_SECTION);
    }
}
