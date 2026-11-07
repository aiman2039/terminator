//! Projects whose folder was moved or deleted, such as a removed Git
//! worktree. The Explorer offers Locate Folder…, Remove Project, and Retry.
//! The session service has no request that changes a project path, so
//! Locate opens the picked folder as a project, selects it, and offers to
//! remove the missing one. When the focused terminal's folder is gone, the
//! Explorer shows the project folder instead. Nothing here runs Git or
//! sends input to a terminal.

use crate::*;
use std::collections::HashMap;

/// The folder pick that locates a missing project, the removal offer that
/// follows it, and terminal folders that are gone.
#[derive(Default)]
pub(crate) struct MissingFolder {
    /// The missing project that the open Locate picker locates.
    pub(crate) locate: Option<Locate>,
    /// A missing project to offer for removal after Locate opened another
    /// folder in its place.
    pub(crate) offer: Option<String>,
    /// Per session: the working folder that a read of the Explorer root
    /// found missing. Kept apart from `directory_errors`, which a new
    /// Explorer root clears, so the root does not go back and forth.
    pub(crate) terminals: HashMap<String, PathBuf>,
}

pub(crate) struct Locate {
    pub(crate) project: String,
    /// Selection generation of the Locate picker. Only the pick of that
    /// picker locates the project.
    pub(crate) generation: u64,
}

/// Sidebar icon of a project whose folder is missing.
pub(crate) const MISSING_ICON: &str = "CircleAlert";
/// First tooltip line of a project whose folder is missing.
pub(crate) const MISSING_TOOLTIP: &str = "Folder not found";

/// The folder itself is gone; it is not a read or access problem.
pub(crate) fn folder_missing(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

pub(crate) fn missing_folder_text(path: &Path) -> String {
    format!(
        "Folder not found: {}. It was moved or deleted.",
        path.display()
    )
}

/// The Explorer note while it shows the project folder in place of the
/// focused terminal's folder.
pub(crate) fn terminal_folder_note(path: &Path) -> String {
    format!(
        "Terminal folder {} was moved or deleted. Showing the project folder.",
        path.display()
    )
}

impl App {
    /// The focused shell whose folder the Explorer shows. A shell whose
    /// folder is gone gives way to the project folder.
    pub(crate) fn folder_session(&self) -> Option<&Session> {
        self.context_session()
            .filter(|session| self.missing_terminal_cwd(session).is_none())
    }

    /// The focused shell's folder, while it is gone and the Explorer shows
    /// the project folder in its place.
    pub(crate) fn missing_terminal_folder(&self) -> Option<&Path> {
        self.missing_terminal_cwd(self.context_session()?)
    }

    fn missing_terminal_cwd<'a>(&self, session: &'a Session) -> Option<&'a Path> {
        (self.missing_folder.terminals.get(&session.id) == Some(&session.cwd))
            .then_some(session.cwd.as_path())
    }

    /// A read of the Explorer root `root` failed. When the folder is gone
    /// and it is the focused shell's folder, not the project folder,
    /// remember that for the shell: `cwd()` then gives the project folder.
    pub(crate) fn note_missing_root(&mut self, root: &Path, error: &services::DirectoryError) {
        if !folder_missing(error.kind)
            || self
                .selected_project()
                .is_none_or(|project| project.path == root)
        {
            return;
        }
        if let Some(session) = self.context_session().filter(|s| s.cwd == root) {
            let id = session.id.clone();
            self.missing_folder.terminals.insert(id, root.to_path_buf());
        }
    }

    /// Forget the folder of a shell that ended or reports another folder.
    pub(crate) fn prune_terminal_folders(&mut self) {
        let sessions = &self.state.sessions;
        self.missing_folder.terminals.retain(|id, cwd| {
            sessions
                .iter()
                .any(|s| &s.id == id && s.lifecycle.live() && &s.cwd == cwd)
        });
    }

    /// Retry and Refresh read the Explorer root again. The focused shell's
    /// folder is read again too, as it can be back.
    pub(crate) fn retry_explorer(&mut self) {
        if let Some(id) = self.context_session().map(|session| session.id.clone()) {
            self.missing_folder.terminals.remove(&id);
        }
        self.refresh_request = None;
    }

    /// The directory probe found no folder at the project path.
    pub(crate) fn project_folder_missing(&self, project: &str) -> bool {
        self.missing_projects.contains(project)
    }

    /// The selected project, when the missing Explorer root `root` is its
    /// folder or the directory probe already reports that folder missing.
    pub(crate) fn missing_root_project(&self, root: &Path) -> Option<String> {
        let project = self.selected_project()?;
        (project.path == root || self.project_folder_missing(&project.id))
            .then(|| project.id.clone())
    }

    /// Locate Folder…: asks for a Locate picker. Its pick, and only its
    /// pick, opens the new location.
    pub(crate) fn locate_missing_folder(&mut self, project: &str) {
        self.request_pick(PickKind::Locate(project.to_owned()));
    }

    /// The Add Project picker opens, for Locate when `locate` names the
    /// missing project: returns its selection generation and dialog title.
    /// An older result for another selection is then stale.
    pub(crate) fn begin_project_pick(&mut self, locate: Option<String>) -> (u64, &'static str) {
        self.selection_generation = self.selection_generation.wrapping_add(1);
        let generation = self.selection_generation;
        let kind = match &locate {
            Some(project) => PickKind::Locate(project.clone()),
            None => PickKind::Project,
        };
        self.missing_folder.locate = locate.map(|project| Locate {
            project,
            generation,
        });
        (generation, kind.title())
    }

    /// A pick that cannot open: the picker was cancelled or the selection
    /// changed while it was open.
    pub(crate) fn drop_folder_pick(&mut self, picked: Option<&Path>) {
        self.missing_folder.locate = None;
        if let Some(path) = picked {
            self.info = Some(format!(
                "{} was not opened because the selected project changed. Choose it again.",
                path.display()
            ));
        }
    }

    /// A pick opened `project`. Once it is selected, a Locate that opened
    /// another folder offers to remove the missing project, and a pick of
    /// the project that was already selected (`reopened`) says so, because
    /// the selection and the Explorer do not change.
    pub(crate) fn finish_folder_pick(&mut self, project: &str, generation: u64, reopened: bool) {
        let located = self
            .missing_folder
            .locate
            .take_if(|locate| locate.generation == generation)
            .map(|locate| locate.project);
        if self.selected.as_deref() != Some(project) {
            return;
        }
        match located {
            Some(missing) if missing != project => {
                if self.has_project(&missing)
                    && !self.preferences.hidden_projects.contains(&missing)
                {
                    self.missing_folder.offer = Some(missing);
                }
            }
            _ if reopened => self.info = Some(self.already_open_text(project)),
            _ => {}
        }
    }

    /// A pick of the selected project changes neither the sidebar nor the
    /// Explorer, so the status bar says so.
    fn already_open_text(&self, project: &str) -> String {
        self.state
            .projects
            .iter()
            .find(|p| p.id == project)
            .map_or_else(
                || "The project is already open.".into(),
                |open| format!("{} is already open.", open.name),
            )
    }

    /// After Locate opened another folder: remove the missing project from
    /// the sidebar (the usual Remove project flow), or keep it.
    pub(crate) fn missing_offer_dialog(&mut self, ctx: &egui::Context) {
        let Some(project) = self.missing_folder.offer.clone() else {
            return;
        };
        let Some(missing) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == project && !self.preferences.hidden_projects.contains(&p.id))
            .cloned()
        else {
            self.missing_folder.offer = None;
            return;
        };
        let mut open = true;
        let mut done = false;
        self.popups
            .window(ctx, "Remove missing project?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} points to {}, which was moved or deleted.",
                    missing.name,
                    missing.path.display()
                ));
                ui.label("Remove it from the sidebar? Its terminals keep running.");
                ui.horizontal(|ui| {
                    let remove = ui.button("Remove Project");
                    #[cfg(feature = "test-support")]
                    diagnostics::record_ui(ui, "missing-offer-remove", remove.rect);
                    if remove.clicked() {
                        self.hide_project(&project);
                        done = true;
                    }
                    if ui.button("Keep").clicked() {
                        done = true;
                    }
                });
            });
        if done || !open {
            self.missing_folder.offer = None;
        }
    }
}
