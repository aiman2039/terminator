//! GUI-only project display overrides: rename, color, and icon.
use super::*;
use crate::preferences::ProjectStyle;
use std::collections::HashMap;
use terminator_core::Project;

const ICONS: &[&str] = &[
    "Folder",
    "FolderCode",
    "FolderGit",
    "Terminal",
    "Monitor",
    "Database",
    "Globe",
    "GitBranch",
    "Bookmark",
    "Pin",
    "Settings",
    "Archive",
    "LayoutDashboard",
    "Smartphone",
    "FileCode",
    "Radio",
];

#[derive(Default)]
pub(crate) struct ProjectNames {
    /// Daemon names before the display overrides, keyed by project ID.
    pub(crate) base: HashMap<String, String>,
    pub(crate) draft: Option<RenameDraft>,
}

/// The sidebar rename field. A draft that did not paint this frame commits
/// at frame end, so a hidden field never blocks input.
pub(crate) struct RenameDraft {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) painted: bool,
}

/// Replace daemon names with the user's display names.
pub(crate) fn apply_project_names(
    projects: &mut [Project],
    styles: &HashMap<String, ProjectStyle>,
) {
    for project in projects {
        if let Some(name) = styles.get(&project.id).and_then(|s| s.name.as_ref()) {
            project.name.clone_from(name);
        }
    }
}

impl App {
    pub(super) fn project_color(&self, id: &str) -> Option<egui::Color32> {
        self.preferences
            .project_styles
            .get(id)
            .and_then(|style| style.color.as_deref())
            .map(appearance::color)
    }

    /// The chosen icon, else `fallback`.
    pub(super) fn project_icon(&self, id: &str, fallback: &'static str) -> &'static str {
        self.preferences
            .project_styles
            .get(id)
            .and_then(|style| style.icon.as_deref())
            .and_then(|icon| ICONS.iter().copied().find(|candidate| *candidate == icon))
            .unwrap_or(fallback)
    }

    fn edit_project_style(&mut self, id: &str, edit: impl FnOnce(&mut ProjectStyle)) {
        let style = self
            .preferences
            .project_styles
            .entry(id.to_owned())
            .or_default();
        let name = style.name.clone();
        edit(style);
        let renamed = style.name != name;
        if *style == ProjectStyle::default() {
            self.preferences.project_styles.remove(id);
        }
        if renamed {
            self.refresh_project_names();
        }
    }

    /// Re-apply display names to the current state. A new vector also
    /// invalidates the sidebar cache.
    fn refresh_project_names(&mut self) {
        let mut projects = self.state.projects.clone();
        for project in &mut projects {
            if let Some(base) = self.project_names.base.get(&project.id) {
                project.name.clone_from(base);
            }
        }
        apply_project_names(&mut projects, &self.preferences.project_styles);
        self.state.projects = projects;
    }

    pub(super) fn begin_project_rename(&mut self, p: &Project) {
        self.finish_rename(true);
        self.project_names.draft = Some(RenameDraft {
            id: p.id.clone(),
            text: p.name.clone(),
            painted: true,
        });
    }

    /// Save a non-empty name; an empty name restores the original.
    pub(crate) fn finish_project_rename(&mut self, save: bool) {
        let Some(RenameDraft { id, text, .. }) = self.project_names.draft.take() else {
            return;
        };
        if !save {
            return;
        }
        let draft = text.trim().to_owned();
        let base = self.project_names.base.get(&id).cloned();
        self.edit_project_style(&id, |style| {
            style.name = (!draft.is_empty() && Some(&draft) != base.as_ref()).then_some(draft);
        });
    }

    /// The inline rename field that replaces the project row.
    pub(super) fn project_rename_field(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.project_names.draft.as_mut() else {
            return;
        };
        draft.painted = true;
        let response = ui.add_sized(
            [ui.available_width(), self.theme.row_height()],
            appearance::singleline(&mut draft.text).id(ui.scope_id().with("project-rename")),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-rename", response.rect);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            self.finish_project_rename(false);
        } else if response.lost_focus() || enter {
            self.finish_project_rename(true);
        } else if !response.has_focus() {
            response.request_focus();
        }
    }

    pub(super) fn project_style_menu(&mut self, ui: &mut egui::Ui, p: &Project) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Rename…",
            icon: "Pencil",
            shortcut: "",
        })
        .clicked()
        {
            self.begin_project_rename(p);
            ui.close();
        }
        appearance::menu_button(ui, "Color", |ui| {
            for &(name, hex) in crate::workspace_ui::TITLE_COLORS {
                if color_button(ui, name, appearance::color(hex)) {
                    self.edit_project_style(&p.id, |s| s.color = Some(hex.into()));
                    ui.close();
                }
            }
            ui.separator();
            if menu_entry(ui, "Default color", "Eraser") {
                self.edit_project_style(&p.id, |s| s.color = None);
                ui.close();
            }
        });
        appearance::menu_button(ui, "Icon", |ui| {
            for &icon in ICONS {
                if menu_entry(ui, icon, icon) {
                    self.edit_project_style(&p.id, |s| s.icon = Some(icon.into()));
                    ui.close();
                }
            }
            ui.separator();
            if menu_entry(ui, "Default icon", "Eraser") {
                self.edit_project_style(&p.id, |s| s.icon = None);
                ui.close();
            }
        });
        if self.preferences.project_styles.contains_key(&p.id)
            && menu_entry(ui, "Reset appearance", "Eraser")
        {
            self.edit_project_style(&p.id, |s| *s = ProjectStyle::default());
            ui.close();
        }
        ui.separator();
    }
}

fn menu_entry(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut: "",
    })
    .clicked()
}

/// A color-named menu button that native fixtures can target by `name`.
fn color_button(ui: &mut egui::Ui, name: &str, color: egui::Color32) -> bool {
    let response = ui.button(egui::RichText::new(name).color(color));
    #[cfg(feature = "test-support")]
    crate::diagnostics::record_ui(ui, name, response.rect);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            path: "/tmp".into(),
            layout: serde_json::Value::Null,
        }
    }

    #[test]
    fn renamed_project_shows_display_name_and_others_keep_theirs() {
        let mut projects = vec![project("a", "alpha"), project("b", "beta")];
        let mut styles = HashMap::new();
        styles.insert(
            "a".to_owned(),
            ProjectStyle {
                name: Some("Main".into()),
                ..ProjectStyle::default()
            },
        );
        apply_project_names(&mut projects, &styles);
        assert_eq!(projects[0].name, "Main");
        assert_eq!(projects[1].name, "beta");
    }
}
