//! Native editor commands in Find Action: the editor they act on, and how
//! one runs there. The palette holds keyboard focus while it is open, so
//! the target is the native editor on screen that had focus last.
use super::*;
use terminator_native_edit::view::{EditorCommand, queue_command};

/// The native editor view that palette editor commands act on.
pub(crate) struct EditorTarget {
    pub(crate) path: PathBuf,
    /// Source view id: whose caret the command uses, and its queue key.
    pub(crate) view: String,
    language: Language,
    read_only: bool,
    vim: bool,
}

impl EditorTarget {
    /// The command works in this editor: a language server where it needs
    /// one, vim off for multiple carets, and no edits while read-only.
    pub(crate) fn offers(&self, command: EditorCommand) -> bool {
        command.available(self.language, self.vim, self.read_only)
    }
}

impl NativeDoc {
    /// The source view draws: the buffer has loaded and Markdown is not
    /// in preview-only mode.
    fn shows_source(&self) -> bool {
        self.doc.is_some()
            && !(markdown::supported(&self.path) && self.markdown_mode == markdown::Mode::Preview)
    }

    /// Mark view `view` as focused at `at`, drawn in the first pass (tests
    /// outside this module).
    #[cfg(test)]
    pub(crate) fn focus_for_test(&mut self, view: &str, at: Instant) {
        self.focused_view = Some(view.to_owned());
        self.focused_at = Some(at);
        self.drawn.insert(view.to_owned(), 0);
    }

    /// The view palette commands go to: the focused one while it is on
    /// screen, else the one that drew last (the focused view may be in a
    /// hidden tab, or gone after a split moved its pane).
    fn command_view(&self) -> Option<String> {
        self.focused_view
            .clone()
            .filter(|view| self.shows_view(view))
            .or_else(|| {
                let (view, _) = self.drawn.iter().max_by_key(|(_, pass)| **pass)?;
                Some(view.clone())
            })
    }
}

impl App {
    /// The editor that palette commands act on: of the native editors on
    /// screen in the selected project, the one focused last, else the
    /// active pane. `None` when no loaded native editor source shows.
    pub(crate) fn editor_command_target(&self) -> Option<EditorTarget> {
        let active = self.active_editor_path_ref();
        let (path, doc) = self
            .visible_native_paths()
            .into_iter()
            .filter_map(|path| {
                let doc = self.native_docs.get(&path)?;
                doc.shows_source().then_some((path, doc))
            })
            .max_by_key(|(path, doc)| (doc.focused_at, active == Some(path.as_path())))?;
        let read_only = self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(&path))
            || crate::remote_ui::is_snapshot(&self.paths.data, &path);
        Some(EditorTarget {
            view: doc.command_view()?,
            language: language_for_path(&path.to_string_lossy()),
            read_only,
            vim: self.state.settings.native_vim,
            path,
        })
    }

    /// Native editors on screen in the selected project: the active tab of
    /// each pane in its current workspace tab and in its floating windows.
    fn visible_native_paths(&self) -> Vec<PathBuf> {
        let Some(project) = self.selected.as_deref() else {
            return Vec::new();
        };
        let floats = self
            .floating
            .iter()
            .filter(|window| window.home.0 == project)
            .filter_map(|window| window.dock.as_ref());
        let docks = self
            .layouts
            .get(project)
            .map(std::ops::Deref::deref)
            .into_iter()
            .chain(floats);
        let mut paths = Vec::new();
        for dock in docks {
            for (_, leaf) in dock.iter_leaves() {
                if let Some(Tab::NativeEditor { path }) = leaf.tabs.get(leaf.active.0)
                    && !paths.contains(path)
                {
                    paths.push(path.clone());
                }
            }
        }
        paths
    }

    /// Run `command` in the target editor: select its pane, then queue the
    /// command for the view's next frame. The view takes keyboard focus.
    pub(crate) fn run_editor_command(&mut self, ctx: &egui::Context, command: EditorCommand) {
        let Some(target) = self.editor_command_target() else {
            self.info = Some(format!(
                "{}: open a file in the editor first",
                command.name()
            ));
            return;
        };
        if !target.offers(command) {
            self.info = Some(format!("{} does not apply to this file", command.name()));
            return;
        }
        if let Some(project) = self.selected.clone() {
            self.focus_file_view(
                &project,
                &Tab::NativeEditor {
                    path: target.path.clone(),
                },
            );
        }
        queue_command(ctx, &target.view, command);
    }
}
