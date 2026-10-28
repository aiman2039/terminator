mod list;
mod tab_tree;

use super::super::agent_presence::attention_status_icon;
use super::attention::state_color;
use super::explorer_rows::ExplorerLocal;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::sidebar_ui::SessionRowInTabInput;
use crate::{
    App, Job, Tab, appearance, icons,
    preferences::{HistorySort, ProjectSort},
    workspace_ops,
};
use eframe::egui::{self};
use std::sync::Arc;
use terminator_core::{AgentState, Session, SessionKind};

/// Outcome of painting one sidebar group header.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupRenameAction {
    None,
    Begin,
    Commit,
    Cancel,
    Clear,
}

impl App {
    fn project_sort_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.preferences.project_sort;
        let menu = appearance::sidebar_action(ui, "ArrowDownWideNarrow", "Sort projects");
        egui::Popup::menu(&menu)
            .style(appearance::menu_style)
            .show(|ui| {
                for (sort, icon, target) in [
                    (ProjectSort::NameAsc, "ArrowDown", "sort-name-asc"),
                    (ProjectSort::NameDesc, "ArrowUp", "sort-name-desc"),
                    (
                        ProjectSort::LatestActivity,
                        "History",
                        "sort-latest-activity",
                    ),
                ] {
                    let check = if current == sort { "✓" } else { "" };
                    let response = appearance::menu_item(ui, sort.menu_label(), icon, check);
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), target, response.rect);
                    #[cfg(not(feature = "test-support"))]
                    let _ = target;
                    if response.clicked() {
                        self.preferences.project_sort = sort;
                        ui.close();
                    }
                }
            });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "project-sort", menu.rect);
        let _ = menu;
    }

    pub(super) fn history_sort_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let current = self.preferences.history_sort;
        let menu = appearance::menu_button(ui, "Sort", |ui| {
            for (sort, icon, target) in [
                (
                    HistorySort::LatestActivity,
                    "History",
                    "history-sort-latest-activity",
                ),
                (HistorySort::NameAsc, "ArrowDown", "history-sort-name-asc"),
                (HistorySort::NameDesc, "ArrowUp", "history-sort-name-desc"),
            ] {
                let check = if current == sort { "✓" } else { "" };
                let response = appearance::menu_item(ui, sort.menu_label(), icon, check);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), target, response.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = target;
                if response.clicked() {
                    self.preferences.history_sort = sort;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Sort History by latest activity or name");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "history-sort", menu.rect);
        let _ = menu;
    }

    fn removed_projects_menu(&mut self, ui: &mut egui::Ui) {
        let hidden: Vec<_> = self
            .state
            .projects
            .iter()
            .filter(|p| self.preferences.hidden_projects.contains(&p.id))
            .cloned()
            .collect();
        if hidden.is_empty() {
            return;
        }
        let menu = appearance::sidebar_action(ui, "Archive", "Removed projects");
        egui::Popup::menu(&menu)
            .style(appearance::menu_style)
            .show(|ui| {
                for project in hidden {
                    let response = appearance::menu_item(ui, &project.name, "FolderOpen", "")
                        .on_hover_text(self.project_location_label(&project.id));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("restore-project:{}", project.id),
                        response.rect,
                    );
                    if response.clicked() {
                        self.select_project(project.id);
                        ui.close();
                    }
                }
            });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "removed-projects", menu.rect);
        let _ = menu;
    }

    pub(super) fn op_root(&self) -> Option<std::path::PathBuf> {
        if self.selected_remote_project().is_some() {
            return None;
        }
        self.git_root()
            .or_else(|| self.cwd())
            .or_else(|| self.selected_project().map(|project| project.path.clone()))
    }

    pub(crate) fn queue_workspace(&mut self, op: workspace_ops::Op) {
        let Some(root) = self.op_root() else {
            self.error = Some("Open a project folder first.".into());
            return;
        };
        let _ = self.jobs.send(Job::Workspace(root, op));
    }

    pub(super) fn explorer_local(
        &mut self,
        path: &std::path::Path,
        local: ExplorerLocal,
        ui: &egui::Ui,
    ) {
        let root = self.op_root().unwrap_or_else(|| path.to_path_buf());
        match local {
            ExplorerLocal::Reveal => self.queue_workspace(workspace_ops::Op::Reveal(path.into())),
            ExplorerLocal::CopyRelative => {
                ui.ctx()
                    .copy_text(workspace_ops::relative_display(&root, path));
            }
            ExplorerLocal::History | ExplorerLocal::CompareWith(_) => {
                let Some(cwd) = self
                    .context
                    .as_ref()
                    .and_then(|context| context.root.clone())
                else {
                    return;
                };
                let tab = match local {
                    ExplorerLocal::CompareWith(branch) => Tab::RevisionDiff {
                        cwd,
                        path: path.to_owned(),
                        left: branch,
                        right: String::new(),
                    },
                    _ => Tab::FileHistory {
                        cwd,
                        path: path.to_owned(),
                    },
                };
                self.open_git_tab(tab);
            }
            ExplorerLocal::CopyName => {
                ui.ctx().copy_text(
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            ExplorerLocal::Rename => self.explorer_rename(path),
            ExplorerLocal::Duplicate => {
                self.queue_workspace(workspace_ops::Op::Duplicate(path.into()));
            }
            ExplorerLocal::Delete => {
                // Deleting a row inside a multi-selection trashes them all.
                if !self.explorer_selection.iter().any(|p| p == path) {
                    self.explorer_selection = vec![path.into()];
                }
                self.explorer_confirm_delete();
            }
        }
    }
}

/// Group live session ids into per-tab trees in layout order.
///
/// Each tab keeps its layout order and only ids present in `live` are kept;
/// tabs with no live sessions are skipped. Ids in no tab are returned as
/// overflow in `live` order (background sessions).
pub(crate) fn group_session_ids_by_tab(
    tabs: &[(String, Vec<String>)],
    live: &[String],
) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    use std::collections::HashSet;
    let known: HashSet<&str> = live.iter().map(String::as_str).collect();
    let mut in_tab: HashSet<&str> = HashSet::new();
    let mut groups = Vec::new();
    for (tab_id, sids) in tabs {
        let kept: Vec<String> = sids
            .iter()
            .filter(|sid| known.contains(sid.as_str()) && in_tab.insert(sid.as_str()))
            .cloned()
            .collect();
        if !kept.is_empty() {
            groups.push((tab_id.clone(), kept));
        }
    }
    let overflow: Vec<String> = live
        .iter()
        .filter(|sid| !in_tab.contains(sid.as_str()))
        .cloned()
        .collect();
    (groups, overflow)
}
