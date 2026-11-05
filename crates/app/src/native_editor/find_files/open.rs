//! Opening Find in Files, or Replace in Files (the same window in replace
//! mode): from a focused native editor (seeded with its selection), from
//! the action anywhere (scoped to a focused Explorer folder, else the
//! project), and from an Explorer folder's menu. Remote projects are not
//! searched; a note says so.
use super::*;

/// Shown instead of a search when a remote project is selected.
fn remote_note(replace: bool) -> &'static str {
    if replace {
        "Replace in Files changes local projects only"
    } else {
        "Find in Files searches local projects only"
    }
}

/// Where Find in Files searches: `root` anchors relative paths and globs;
/// `folder` narrows the walk to one directory below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindScope {
    pub root: PathBuf,
    pub folder: Option<PathBuf>,
}

impl FindScope {
    /// The project, narrowed to `folder` when it lies inside it; a folder
    /// elsewhere is its own root. `None` with neither.
    pub(crate) fn resolve(project: Option<&Path>, folder: Option<&Path>) -> Option<Self> {
        match (project, folder) {
            (Some(root), Some(folder)) if folder.starts_with(root) => Some(Self {
                root: root.to_owned(),
                folder: (folder != root).then(|| folder.to_owned()),
            }),
            (_, Some(folder)) => Some(Self {
                root: folder.to_owned(),
                folder: None,
            }),
            (Some(root), None) => Some(Self {
                root: root.to_owned(),
                folder: None,
            }),
            (None, None) => None,
        }
    }
}

/// Temp-data key of the Explorer tree that last held keyboard focus.
fn explorer_focus_key() -> egui::Id {
    egui::Id::unique("find-in-files-explorer-focus")
}

/// Note a focused Explorer tree, so the Find in Files action searches its
/// selected folder while it keeps focus.
pub(crate) fn note_explorer_focus(ctx: &egui::Context, focus: egui::Id) {
    ctx.data_mut(|data| data.insert_temp(explorer_focus_key(), focus));
}

/// The Explorer tree holds keyboard focus.
pub(crate) fn explorer_has_focus(ctx: &egui::Context) -> bool {
    let noted = ctx.data(|data| data.get_temp::<egui::Id>(explorer_focus_key()));
    noted.is_some() && ctx.memory(|memory| memory.focused()) == noted
}

impl App {
    /// Find in Files from a native editor (its key or menu), seeded with
    /// its one-line selection.
    pub(crate) fn open_find_in_files(&mut self, project: Option<&str>, seed: String) {
        self.open_search_in_files(project, seed, false);
    }

    /// Replace in Files from a native editor, seeded like
    /// [`Self::open_find_in_files`].
    pub(crate) fn open_replace_in_files(&mut self, project: Option<&str>, seed: String) {
        self.open_search_in_files(project, seed, true);
    }

    fn open_search_in_files(&mut self, project: Option<&str>, seed: String, replace: bool) {
        if project.is_some_and(|id| self.state.remote_projects.iter().any(|p| p.id == id)) {
            self.info = Some(remote_note(replace).into());
            return;
        }
        let root = project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|project| project.path.clone())
            .or_else(|| self.local_project_root());
        let folder = self.open_folder(root.as_deref());
        let scope = FindScope::resolve(root.as_deref(), folder.as_deref());
        self.show_find_in_files(scope, seed, replace);
    }

    /// The Find in Files action (shortcut or palette). A focused Explorer
    /// folder narrows it; otherwise it searches the project, seeded with
    /// the active native editor's selection. An open window keeps its
    /// query and folder and only takes the keyboard back.
    pub(crate) fn find_in_files_action(&mut self, ctx: &egui::Context) {
        self.search_in_files_action(ctx, false);
    }

    /// The Replace in Files action: [`Self::find_in_files_action`] in
    /// replace mode.
    pub(crate) fn replace_in_files_action(&mut self, ctx: &egui::Context) {
        self.search_in_files_action(ctx, true);
    }

    fn search_in_files_action(&mut self, ctx: &egui::Context, replace: bool) {
        let explorer = self.explorer_focused_folder(ctx);
        let shown = self.find_in_files.as_ref().is_some_and(|find| find.shown);
        let seed = if explorer.is_some() || shown {
            String::new()
        } else {
            self.active_editor_selection()
        };
        let root = self.local_project_root();
        let folder = explorer.or_else(|| self.open_folder(root.as_deref()));
        let scope = FindScope::resolve(root.as_deref(), folder.as_deref());
        self.show_find_in_files(scope, seed, replace);
    }

    /// The folder of an open window on `root`: asking again keeps it.
    fn open_folder(&self, root: Option<&Path>) -> Option<PathBuf> {
        self.find_in_files
            .as_ref()
            .filter(|find| find.shown && root == Some(find.root.as_path()))
            .and_then(|find| find.folder.clone())
    }

    /// Explorer "Find in Folder…": Find in Files limited to `folder`.
    pub(crate) fn open_find_in_folder(&mut self, folder: &Path) {
        let root = self.local_project_root();
        let scope = FindScope::resolve(root.as_deref(), Some(folder));
        self.show_find_in_files(scope, String::new(), false);
    }

    /// Explorer "Replace in Folder…": Replace in Files limited to `folder`.
    pub(crate) fn open_replace_in_folder(&mut self, folder: &Path) {
        let root = self.local_project_root();
        let scope = FindScope::resolve(root.as_deref(), Some(folder));
        self.show_find_in_files(scope, String::new(), true);
    }

    fn show_find_in_files(&mut self, scope: Option<FindScope>, seed: String, replace: bool) {
        let Some(scope) = scope else {
            self.info = Some(if self.selected_remote_project().is_some() {
                remote_note(replace).into()
            } else if replace {
                "Replace in Files needs an open project".into()
            } else {
                "Find in Files needs an open project".into()
            });
            return;
        };
        let find = match self.find_in_files.as_mut() {
            Some(find) => {
                find.reopen(scope, seed);
                find
            }
            None => self.find_in_files.insert(FindInFiles::new(scope, seed)),
        };
        find.set_replace_mode(replace);
    }

    fn local_project_root(&self) -> Option<PathBuf> {
        self.selected_project().map(|project| project.path.clone())
    }

    /// The last selected Explorer row when it is a folder and its tree
    /// holds keyboard focus.
    fn explorer_focused_folder(&self, ctx: &egui::Context) -> Option<PathBuf> {
        if !explorer_has_focus(ctx) {
            return None;
        }
        let path = self.explorer_selection.last()?;
        let listed_folder = path
            .parent()
            .and_then(|parent| self.dirs.get(parent))
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry.directory && entry.path == *path)
            });
        (listed_folder || self.dirs.contains_key(path)).then(|| path.clone())
    }

    /// The active native editor's selection when it is one line.
    fn active_editor_selection(&self) -> String {
        let Some(doc) = self
            .active_editor_path_ref()
            .and_then(|path| self.native_docs.get(path))
        else {
            return String::new();
        };
        let text = doc
            .doc
            .as_ref()
            .filter(|_| doc.engine.selection().is_some())
            .map(|buffer| doc.engine.copy_text(buffer))
            .unwrap_or_default();
        if text.contains('\n') {
            String::new()
        } else {
            text
        }
    }
}
