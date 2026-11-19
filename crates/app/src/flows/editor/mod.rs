//! The native flow editor: a workspace tab (`Tab::Document` with
//! `DocumentKind::Flow`) with a node graph (`egui-snarl`). Each node is a
//! step or a gate, and a wire is `needs`. Save checks the graph with the
//! flow file's own rules and writes `NAME.toml` on a worker thread; a wrong
//! flow shows its error and writes nothing. The editor never runs a flow
//! and never types into a terminal.
//!
//! A flow with changes that are not saved stays in GUI memory when its
//! tab closes: the same flow opens again with those changes.
mod banner;
mod canvas;
mod cards;
mod dialogs;
mod failed;
mod inspector;
mod keys;
mod model;
mod quit;
mod stay;
mod style;
mod tab;
#[cfg(test)]
mod tests;
mod toolbar;
mod view;
#[cfg(test)]
mod view_tests;
mod viewer;

use crate::document_view::DocumentKind;
use crate::*;
pub(crate) use banner::Starting;
use egui_snarl::{NodeId, Snarl};
pub(crate) use failed::Failure;
pub(crate) use keys::FLOW_ACTIONS;
use model::Node;
pub(crate) use quit::Quit;
#[cfg(test)]
use stay::STAY_GRACE;
pub(crate) use stay::Stay;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
pub(crate) use tab::FLOW_TAB_ICON;
use terminator_core::flow::{Draft, Flow, files};

/// One open flow in the editor. GUI memory only.
pub(crate) struct FlowEditor {
    root: PathBuf,
    /// The file name without `.toml`.
    file: String,
    /// The flow's display name.
    title: String,
    /// The flow's rules: one text that every step gets before its prompt.
    rules: String,
    /// False for a new flow that has no file yet.
    on_disk: bool,
    snarl: Snarl<Node>,
    /// The draft at open or at the last save.
    saved: Draft,
    error: Option<String>,
    /// The draft that a worker thread writes now, and its result.
    saving: Option<(Draft, Receiver<Result<PathBuf, String>>)>,
    /// The plan file that a worker thread writes now, and its result.
    exporting: Option<Receiver<Result<PathBuf, String>>>,
    /// The next frame moves the view so that all nodes show.
    fit: bool,
    /// The measured size of each card, from the last frame.
    sizes: canvas::Sizes,
    /// The canvas size of the last frame: a new size fits the view again.
    canvas: Option<egui::Vec2>,
    /// The card that the inspector edits.
    selected: Option<NodeId>,
    /// The cards that egui-snarl selected (Shift+click or a drag box).
    marked: Vec<NodeId>,
    /// The flow name shows as a field.
    editing_title: bool,
    /// The inspector shows the rules, also on a narrow pane.
    rules_open: bool,
    /// Cancel run waits for its confirmation, opened at this point.
    confirm_cancel: Option<egui::Pos2>,
    dialog: Option<Dialog>,
    /// The "closed with changes" note was shown.
    warned: bool,
    /// Render state of the plan preview's Markdown view.
    preview_cache: egui_commonmark::CommonMarkCache,
}

/// The dialog of an editor: one at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialog {
    /// The plan as Markdown and the flow file text.
    Preview,
}

/// A worker thread's read of a flow file.
pub(crate) enum Read {
    Flow(Flow),
    /// No file: a new flow.
    Missing,
    Failed(Failure),
}

/// The editor of one flow file path.
pub(crate) enum Slot {
    Loading(Receiver<Read>),
    Ready(Box<FlowEditor>),
    /// The file is not a correct flow.
    Failed(Failure),
}

const OUTSIDE: &str = "This file is not in a project's .terminator/flows folder.";
const UNREAD: &str = "Could not read the flow file.";

/// The project folder and the flow name of a flow file path.
fn place_of(path: &Path) -> Option<(PathBuf, String)> {
    let file = path.file_stem()?.to_str()?.to_owned();
    let root = path.parent()?.parent()?.parent()?.to_path_buf();
    (files::path_of(&root, &file).ok()? == path).then_some((root, file))
}

impl FlowEditor {
    fn with(root: PathBuf, file: &str, snarl: Snarl<Node>) -> Self {
        Self {
            saved: model::draft_of(file, &snarl),
            root,
            file: file.to_owned(),
            title: file.to_owned(),
            rules: String::new(),
            on_disk: false,
            snarl,
            error: None,
            saving: None,
            exporting: None,
            fit: true,
            sizes: canvas::Sizes::new(),
            canvas: None,
            selected: None,
            marked: Vec::new(),
            editing_title: false,
            rules_open: false,
            confirm_cancel: None,
            dialog: None,
            warned: false,
            preview_cache: egui_commonmark::CommonMarkCache::default(),
        }
    }

    fn new(root: PathBuf, file: &str) -> Self {
        Self::with(root, file, model::starter())
    }

    fn of_flow(root: PathBuf, file: &str, flow: &Flow) -> Self {
        let mut editor = Self::with(root, file, model::graph_of(flow));
        editor.title.clone_from(&flow.name);
        editor.rules = flow.rules.clone().unwrap_or_default();
        editor.on_disk = true;
        editor.saved = editor.draft();
        editor
    }

    fn draft(&self) -> Draft {
        Draft {
            rules: Some(self.rules.trim().to_owned()).filter(|text| !text.is_empty()),
            ..model::draft_of(&self.title, &self.snarl)
        }
    }

    /// The flow as a plan in Markdown.
    fn plan(&self) -> String {
        model::plan_text(&self.draft(), &self.file)
    }

    /// A new flow, or a flow with a change since the last save: content or
    /// a card's place. Save is on for both.
    pub(crate) fn dirty(&self) -> bool {
        !self.on_disk || self.draft() != self.saved
    }

    /// A new flow, or a flow whose content differs from the saved file.
    /// A card's place (`at`) is display only: a moved card alone is not a
    /// change here, so it does not stop Run or a quit.
    pub(crate) fn changed(&self) -> bool {
        !self.on_disk || model::content_of(self.draft()) != model::content_of(self.saved.clone())
    }

    /// Save: the graph must be a correct flow, then a worker thread writes
    /// the file. A new flow never replaces a file.
    fn save(&mut self) {
        if self.saving.is_some() {
            return;
        }
        let draft = self.draft();
        if let Err(error) = draft.check() {
            self.error = Some(error.to_string());
            return;
        }
        let (root, file, replace) = (self.root.clone(), self.file.clone(), self.on_disk);
        let (sender, receiver) = channel();
        let write = draft.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-save".into())
            .spawn(move || {
                let saved = files::save(&root, &file, &write, replace);
                let _ = sender.send(saved.map_err(|error| error.to_string()));
            });
        if spawned.is_ok() {
            self.error = None;
            self.saving = Some((draft, receiver));
        } else {
            self.error = Some("Could not start the save.".into());
        }
    }

    /// Export: a worker thread writes the plan as `NAME.md` beside the
    /// flow file. It replaces only that plan file.
    fn export_plan(&mut self) {
        if self.exporting.is_some() {
            return;
        }
        let text = self.plan();
        let Ok(flow) = files::path_of(&self.root, &self.file) else {
            return;
        };
        let path = flow.with_extension("md");
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-export".into())
            .spawn(move || {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, text));
                let _ = sender.send(written.map(|()| path).map_err(|error| error.to_string()));
            });
        if spawned.is_ok() {
            self.exporting = Some(receiver);
        } else {
            self.error = Some("Could not start the export.".into());
        }
    }

    /// Takes the result of an export: the written plan file.
    fn poll_export(&mut self) -> Option<PathBuf> {
        match self.exporting.as_ref()?.try_recv() {
            Ok(Ok(path)) => {
                self.exporting = None;
                Some(path)
            }
            Ok(Err(error)) => {
                self.error = Some(format!("Export: {error}"));
                self.exporting = None;
                None
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.exporting = None;
                None
            }
        }
    }

    /// Takes the result of a save. Gives a note for the status bar.
    fn poll(&mut self) -> Option<String> {
        let (draft, result) = self.saving.as_ref()?;
        match result.try_recv() {
            Ok(Ok(_)) => {
                self.saved = draft.clone();
                self.on_disk = true;
                self.error = None;
                self.saving = None;
                Some(format!("Saved the flow {}.", self.file))
            }
            Ok(Err(error)) => {
                self.error = Some(error);
                self.saving = None;
                None
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.error = Some("The save did not finish.".into());
                self.saving = None;
                None
            }
        }
    }
}

impl Slot {
    /// Reads the flow of `path` on a worker thread.
    fn load(path: &Path) -> Self {
        let Some((root, file)) = place_of(path) else {
            return Self::Failed(Failure::plain(OUTSIDE));
        };
        let (sender, receiver) = channel();
        let path = path.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("flow-open".into())
            .spawn(move || {
                let read = match files::load(&root, &file) {
                    Ok(flow) => Read::Flow(flow),
                    Err(files::LoadError::Missing(_)) => Read::Missing,
                    Err(error) => Read::Failed(failed::failure_of(&path, &error)),
                };
                let _ = sender.send(read);
            });
        match spawned {
            Ok(_) => Self::Loading(receiver),
            Err(_) => Self::Failed(Failure::plain(UNREAD)),
        }
    }

    /// The editor of a read result. A flow with no file is a new flow.
    fn loaded(path: &Path, read: Read) -> Self {
        let Some((root, file)) = place_of(path) else {
            return Self::Failed(Failure::plain(OUTSIDE));
        };
        match read {
            Read::Flow(flow) => Self::Ready(Box::new(FlowEditor::of_flow(root, &file, &flow))),
            Read::Missing => Self::Ready(Box::new(FlowEditor::new(root, &file))),
            Read::Failed(failure) => Self::Failed(failure),
        }
    }

    /// Takes the read result when it is there.
    fn poll(self, path: &Path) -> Self {
        let Self::Loading(receiver) = self else {
            return self;
        };
        match receiver.try_recv() {
            Ok(read) => Self::loaded(path, read),
            Err(TryRecvError::Empty) => Self::Loading(receiver),
            Err(TryRecvError::Disconnected) => Self::Failed(Failure::plain(UNREAD)),
        }
    }

    fn keep(&self) -> bool {
        matches!(self, Self::Ready(editor) if editor.dirty() || editor.saving.is_some())
    }
}

impl App {
    /// Opens flow `file` of the project at `root` in an editor tab. A name
    /// with no file opens as a new flow; nothing is written until Save.
    /// False for a name that is not a flow name.
    pub(crate) fn open_flow_tab(&mut self, project: &str, root: &Path, file: &str) -> bool {
        let Ok(path) = files::path_of(root, file.trim()) else {
            self.info = Some("A flow name must be letters, digits, '-' or '_'.".into());
            return false;
        };
        if !self.project_available(project) {
            self.info = Some("The flow's project is not open.".into());
            return false;
        }
        if self.selected.as_deref() != Some(project) {
            self.select_project(project.to_owned());
        }
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), None);
        let tab = Tab::Document {
            path,
            kind: DocumentKind::Flow,
        };
        let _ = self
            .update_tx
            .send(Update::OpenDocument(project.into(), tab, after));
        true
    }

    /// A run reads the flow file on disk. When an open editor of that file
    /// has content that is not saved, the run does not start: the editor's
    /// tab shows with a note. A moved card alone does not stop a run.
    pub(crate) fn flow_run_waits_for_save(
        &mut self,
        loaded: &super::LoadedFlows,
        file: &str,
    ) -> bool {
        let Ok(path) = files::path_of(&loaded.root, file) else {
            return false;
        };
        if !matches!(self.flows.editors.get(&path), Some(Slot::Ready(editor)) if editor.changed()) {
            return false;
        }
        self.open_flow_tab(&loaded.project, &loaded.root, file);
        self.info = Some(format!(
            "The flow {file} has changes that are not saved. Save it, then run."
        ));
        true
    }

    /// Run from the Flows window: opens the flow's editor tab, starts the
    /// run and stays on that tab, because its cards show the run. False
    /// when the run did not start.
    pub(crate) fn run_flow_in_editor(&mut self, loaded: &super::LoadedFlows, file: &str) -> bool {
        let Some(runner) = self.run_flow(loaded, file) else {
            return false;
        };
        if self.open_flow_tab(&loaded.project, &loaded.root, file)
            && let Ok(path) = files::path_of(&loaded.root, file)
        {
            self.flows.stay = Some(Stay::after_run(&path, runner, &self.flows.runs));
            self.flows.starting = Some(Starting::now(&path));
        }
        self.info = Some(format!(
            "Started the flow {file}. Its tab shows each step; a step opens its own terminal."
        ));
        true
    }

    /// After the panes of a frame: takes save results of editors that no
    /// pane painted, and drops editors with no changes and no pane. An
    /// editor with changes stays; one note says so when its tab is gone.
    pub(crate) fn flow_editors_end_frame(&mut self) {
        self.flow_stay_on_editor();
        if self.flows.editors.is_empty() {
            return;
        }
        let visible = std::mem::take(&mut self.flows.visible);
        self.flows
            .editors
            .retain(|path, slot| visible.contains(path) || slot.keep());
        let hidden: Vec<PathBuf> = self
            .flows
            .editors
            .keys()
            .filter(|path| !visible.contains(*path))
            .cloned()
            .collect();
        self.flows.visible = visible;
        if hidden.is_empty() {
            return;
        }
        let open: Vec<PathBuf> = self
            .open_tabs()
            .filter_map(|(_, tab)| match tab {
                Tab::Document {
                    path,
                    kind: DocumentKind::Flow,
                } => Some(path.clone()),
                _ => None,
            })
            .collect();
        for path in hidden {
            let Some(Slot::Ready(editor)) = self.flows.editors.get_mut(&path) else {
                continue;
            };
            if let Some(note) = editor.poll() {
                self.info = Some(note);
            }
            let closed = !open.contains(&path);
            if closed && editor.changed() && !editor.warned {
                editor.warned = true;
                self.info = Some(format!(
                    "The flow {} has changes that are not saved. Edit it again to save them.",
                    editor.file
                ));
            }
        }
    }
}
