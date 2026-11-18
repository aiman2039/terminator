//! The native flow editor: a center page with a node graph (`egui-snarl`).
//! Each node is a step or a gate, and a wire is `needs`. Save checks the
//! graph with the flow file's own rules and writes `NAME.toml` on a worker
//! thread; a wrong flow shows its error and writes nothing. The editor
//! never runs a flow and never types into a terminal.
mod model;
#[cfg(test)]
mod tests;
mod viewer;

use crate::*;
use egui_snarl::{Snarl, ui::SnarlWidget};
use model::Node;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use terminator_core::flow::{Draft, Flow, files};

/// One open flow in the editor. GUI memory only: it stays when the page
/// hides, until Save and Close or Discard.
pub(crate) struct FlowEditor {
    root: PathBuf,
    /// The file name without `.toml`. Fixed after the first save.
    file: String,
    /// The flow's display name.
    title: String,
    /// False for a new flow that has no file yet.
    on_disk: bool,
    snarl: Snarl<Node>,
    /// The draft at open or at the last save.
    saved: Draft,
    error: Option<String>,
    /// The draft that a worker thread writes now, and its result.
    saving: Option<(Draft, Receiver<Result<PathBuf, String>>)>,
    /// Close was clicked with changes that are not saved.
    confirm_discard: bool,
    /// The next frame moves the view so that all nodes show.
    fit: bool,
}

/// A flow file that a worker thread reads now for the editor.
pub(crate) struct OpeningFlow {
    project: String,
    root: PathBuf,
    file: String,
    result: Receiver<Result<Flow, String>>,
}

impl FlowEditor {
    fn new(root: PathBuf, file: &str) -> Self {
        let snarl = model::starter();
        Self {
            saved: model::draft_of(file, &snarl),
            root,
            file: file.to_owned(),
            title: file.to_owned(),
            on_disk: false,
            snarl,
            error: None,
            saving: None,
            confirm_discard: false,
            fit: true,
        }
    }

    fn of_flow(root: PathBuf, file: &str, flow: &Flow) -> Self {
        let snarl = model::graph_of(flow);
        Self {
            saved: model::draft_of(&flow.name, &snarl),
            root,
            file: file.to_owned(),
            title: flow.name.clone(),
            on_disk: true,
            snarl,
            error: None,
            saving: None,
            confirm_discard: false,
            fit: true,
        }
    }

    fn draft(&self) -> Draft {
        model::draft_of(&self.title, &self.snarl)
    }

    /// A new flow, or a flow with a change since the last save.
    pub(crate) fn dirty(&self) -> bool {
        !self.on_disk || self.draft() != self.saved
    }

    pub(crate) fn file(&self) -> &str {
        &self.file
    }
}

impl App {
    /// Another flow cannot open while the editor has changes: they would
    /// be lost.
    fn editor_blocks_open(&mut self) -> bool {
        let Some(editor) = &self.flows.editor else {
            return false;
        };
        if !editor.dirty() {
            return false;
        }
        self.info = Some(format!(
            "The flow {} has changes. Save or close it first.",
            editor.file
        ));
        self.show_center(CenterPage::FlowEditor);
        true
    }

    /// Opens the editor with a new flow named `file`. Nothing is written
    /// until Save.
    pub(crate) fn new_flow_editor(&mut self, root: PathBuf, file: &str) {
        let file = file.trim();
        if !files::plain_name(file) {
            self.info = Some("A flow name must be letters, digits, '-' or '_'.".into());
            return;
        }
        if self.editor_blocks_open() {
            return;
        }
        self.flows.editor = Some(FlowEditor::new(root, file));
        self.show_center(CenterPage::FlowEditor);
    }

    /// Reads flow `file` on a worker thread; the editor opens when it is
    /// there.
    pub(crate) fn edit_flow_editor(&mut self, project: &str, root: PathBuf, file: &str) {
        if self.editor_blocks_open() {
            return;
        }
        let (sender, receiver) = channel();
        let (read_root, name) = (root.clone(), file.to_owned());
        let spawned = std::thread::Builder::new()
            .name("flow-open".into())
            .spawn(move || {
                let flow = files::load(&read_root, &name).map_err(|error| error.to_string());
                let _ = sender.send(flow);
            });
        if spawned.is_err() {
            self.info = Some("Could not read the flow file.".into());
            return;
        }
        self.flows.opening = Some(OpeningFlow {
            project: project.to_owned(),
            root,
            file: file.to_owned(),
            result: receiver,
        });
    }

    /// Takes the results of the worker threads: an opened flow and a save.
    pub(crate) fn poll_flow_editor(&mut self, ctx: &egui::Context) {
        if let Some(opening) = &self.flows.opening {
            match opening.result.try_recv() {
                Ok(read) => {
                    let opening = self.flows.opening.take();
                    if let Some(opening) = opening {
                        self.opened_flow(opening, read);
                    }
                }
                Err(_) => ctx.request_repaint(),
            }
        }
        let Some(editor) = &mut self.flows.editor else {
            return;
        };
        let Some((draft, result)) = &editor.saving else {
            return;
        };
        match result.try_recv() {
            Ok(Ok(_)) => {
                editor.saved = draft.clone();
                editor.on_disk = true;
                editor.error = None;
                editor.saving = None;
                self.info = Some(format!("Saved the flow {}.", editor.file));
            }
            Ok(Err(error)) => {
                editor.error = Some(error);
                editor.saving = None;
            }
            Err(_) => ctx.request_repaint(),
        }
    }

    /// A flow that reads as correct opens in the editor. A wrong file
    /// opens as text, so the user can repair it.
    fn opened_flow(&mut self, opening: OpeningFlow, read: Result<Flow, String>) {
        match read {
            Ok(flow) => {
                self.flows.editor = Some(FlowEditor::of_flow(opening.root, &opening.file, &flow));
                self.show_center(CenterPage::FlowEditor);
            }
            Err(error) => {
                self.error.set(format!("Flow {}: {error}", opening.file));
                if let Ok(path) = files::path_of(&opening.root, &opening.file) {
                    self.open_flow_as_text(&opening.project, &path);
                }
            }
        }
    }

    /// Save: the graph must be a correct flow, then a worker thread writes
    /// the file. A new flow never replaces a file.
    fn save_flow_editor(&mut self) {
        let Some(editor) = &mut self.flows.editor else {
            return;
        };
        if editor.saving.is_some() {
            return;
        }
        let draft = editor.draft();
        if let Err(error) = draft.clone().check() {
            editor.error = Some(error.to_string());
            return;
        }
        let (root, file, replace) = (editor.root.clone(), editor.file.clone(), editor.on_disk);
        let (sender, receiver) = channel();
        let write = draft.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-save".into())
            .spawn(move || {
                let saved = files::save(&root, &file, &write, replace);
                let _ = sender.send(saved.map_err(|error| error.to_string()));
            });
        if spawned.is_ok() {
            editor.error = None;
            editor.saving = Some((draft, receiver));
        } else {
            editor.error = Some("Could not start the save.".into());
        }
    }

    /// Close: with changes, the first click asks; Discard closes.
    fn close_flow_editor(&mut self, discard: bool) {
        let Some(editor) = &mut self.flows.editor else {
            self.close_center_page(&CenterPage::FlowEditor);
            return;
        };
        if editor.dirty() && !discard {
            editor.confirm_discard = true;
            return;
        }
        self.flows.editor = None;
        self.close_center_page(&CenterPage::FlowEditor);
    }

    /// The editor page.
    pub(crate) fn flow_editor_center(&mut self, ui: &mut egui::Ui) {
        self.poll_flow_editor(ui.ctx());
        let style = appearance::CenterCardStyle::new(&self.theme);
        let colors = viewer::Colors {
            canvas: appearance::color(&self.theme.window),
            node: appearance::color(&self.theme.surface),
            border: appearance::color(&self.theme.border),
            step: appearance::color(&self.theme.status_running),
            gate: appearance::color(&self.theme.status_waiting),
        };
        let mut click = None;
        let ((), dismissed) = appearance::center_card(ui, style, |ui| {
            ui.set_min_size(ui.available_size());
            let Some(editor) = &mut self.flows.editor else {
                ui.weak("No flow is open. Use Run flow… to make or edit one.");
                return;
            };
            click = editor_bar(ui, editor);
            if let Some(error) = &editor.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.weak(
                "Drag from the right dot of a step to the left dot of the next step, \
                 or use + in After. Right-click the canvas, a step or a wire for more.",
            );
            ui.add_space(4.0);
            let fit = std::mem::take(&mut editor.fit).then(|| ui.available_rect_before_wrap());
            SnarlWidget::new()
                .id_salt("flow-editor")
                .style(viewer::style(colors))
                .show(&mut editor.snarl, &mut viewer::Viewer { colors, fit }, ui);
        });
        match click {
            Some(Click::Save) => self.save_flow_editor(),
            Some(Click::Close) => self.close_flow_editor(false),
            Some(Click::Discard) => self.close_flow_editor(true),
            Some(Click::Keep) => {
                if let Some(editor) = &mut self.flows.editor {
                    editor.confirm_discard = false;
                }
            }
            None => {}
        }
        // A click on the margin hides the page; the editor keeps its flow.
        if dismissed {
            self.hide_center_overlay();
        }
    }
}

/// One click in the editor's top bar.
enum Click {
    Save,
    Close,
    Discard,
    Keep,
}

/// The top bar: the flow's file and name, Add step, Add gate, Save, Close.
fn editor_bar(ui: &mut egui::Ui, editor: &mut FlowEditor) -> Option<Click> {
    let mut click = None;
    ui.horizontal(|ui| {
        ui.strong(format!("Flow {}.toml", editor.file));
        ui.add(
            egui::TextEdit::singleline(&mut editor.title)
                .desired_width(180.0)
                .hint_text("Name"),
        );
        if ui.button("Add step").clicked() {
            let id = model::free_id(&editor.snarl, "step");
            let at = next_place(&editor.snarl);
            editor.snarl.insert_node(at, Node::step(&id, "claude", ""));
        }
        if ui.button("Add gate").clicked() {
            let id = model::free_id(&editor.snarl, "gate");
            let at = next_place(&editor.snarl);
            editor.snarl.insert_node(at, Node::gate(&id, ""));
        }
        let saving = editor.saving.is_some();
        let save = ui
            .add_enabled(editor.dirty() && !saving, egui::Button::new("Save"))
            .on_disabled_hover_text(if saving { "Saving…" } else { "No changes" });
        if save.clicked() {
            click = Some(Click::Save);
        }
        if editor.confirm_discard {
            ui.label("Discard the changes?");
            if appearance::danger_button(ui, "Discard").clicked() {
                click = Some(Click::Discard);
            }
            if ui.button("Keep editing").clicked() {
                click = Some(Click::Keep);
            }
        } else if ui.button("Close").clicked() {
            click = Some(Click::Close);
        }
    });
    click
}

/// Where a node from the top bar goes: below the lowest node.
fn next_place(snarl: &Snarl<Node>) -> egui::Pos2 {
    let lowest = snarl
        .nodes_pos()
        .map(|(position, _)| position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    if lowest.is_finite() {
        egui::pos2(0.0, lowest + 260.0)
    } else {
        egui::Pos2::ZERO
    }
}
