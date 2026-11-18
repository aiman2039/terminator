//! The Flows window: the project's flow files with Run and Edit, and New
//! flow. A new flow is a drawing made from a template; New flow and Edit
//! open it in an Excalidraw tab. Making or editing a file needs no trust
//! answer; Run does.
use super::{FlowFile, LoadedFlows};
use crate::app::OpenFileInput;
use crate::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use terminator_core::flow::{drawing, files};

/// One click in the Flows window.
pub(super) enum Pick {
    Run(usize),
    Edit(usize),
    New,
    Trust(bool),
}

/// Writes the drawing of a new flow from the template. Blocking: call it
/// on a worker thread. It never replaces a file.
pub(super) fn write_new_flow(root: &Path, name: &str) -> Result<PathBuf, String> {
    files::new_drawing(root, name).map_err(|error| error.to_string())
}

impl App {
    /// Makes the drawing of flow `name` on a worker thread; the Flows
    /// window opens it when it is there.
    pub(super) fn create_flow(&mut self, loaded: &LoadedFlows, name: &str) {
        let name = name.trim().to_owned();
        if !files::plain_name(&name) {
            self.info = Some("A flow name must be letters, digits, '-' or '_'.".into());
            return;
        }
        let root = loaded.root.clone();
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-new".into())
            .spawn(move || {
                let _ = sender.send(write_new_flow(&root, &name));
            });
        if spawned.is_ok() {
            self.flows.creating = Some((loaded.project.clone(), receiver));
        } else {
            self.info = Some("Could not make the flow file.".into());
        }
    }

    /// Opens the new flow's drawing when the worker thread made it.
    fn poll_created_flow(&mut self) {
        let Some((project, receiver)) = &self.flows.creating else {
            return;
        };
        let Ok(made) = receiver.try_recv() else {
            return;
        };
        let project = project.clone();
        self.flows.creating = None;
        match made {
            Ok(path) => {
                self.flows.new_name.clear();
                self.flows.open = false;
                self.edit_flow(&project, &path);
            }
            Err(error) => self.error.set(format!("New flow: {error}")),
        }
    }

    /// Opens a flow file to change it: a drawing in an Excalidraw tab, a
    /// TOML file in the editor.
    pub(super) fn edit_flow(&mut self, project: &str, path: &Path) {
        if !self.project_available(project) {
            self.info = Some("The flow's project is not open.".into());
            return;
        }
        if self.selected.as_deref() != Some(project) {
            self.select_project(project.to_owned());
        }
        let drawn = path
            .extension()
            .is_some_and(|ext| ext == drawing::EXTENSION);
        if drawn && self.can_draw() {
            self.open_html(project, path.to_path_buf(), None);
        } else {
            self.open_file(OpenFileInput {
                path: path.to_path_buf(),
                line: None,
                split: None,
                external: false,
            });
        }
    }

    /// The Flows window.
    pub(crate) fn flow_dialog(&mut self, ctx: &egui::Context) {
        self.poll_created_flow();
        if !self.flows.open {
            return;
        }
        if let Some(receiver) = &self.flows.loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.flows.loading = None;
            self.flows.loaded = Some(loaded);
        }
        let mut open = true;
        let mut pick = None;
        let mut name = std::mem::take(&mut self.flows.new_name);
        self.popups
            .window(ctx, "Flows")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                let Some(loaded) = &self.flows.loaded else {
                    ui.weak("Reading flow files…");
                    return;
                };
                let answer = self.lsp.trust_answer(&loaded.root);
                pick = flow_trust_bar(ui, loaded, answer)
                    .map(Pick::Trust)
                    .or(flow_rows(ui, loaded, answer))
                    .or(new_flow_row(ui, &mut name));
            });
        self.flows.new_name = name;
        if self.flows.loading.is_some() || self.flows.creating.is_some() {
            ctx.request_repaint();
        }
        let loaded = self.flows.loaded.clone().unwrap_or_default();
        match pick {
            Some(Pick::Trust(trusted)) => self.answer_task_trust(&loaded.root, trusted),
            Some(Pick::Run(index)) => {
                if let Some(file) = loaded.files.get(index) {
                    self.run_flow(&loaded, &file.file);
                    open = false;
                }
            }
            Some(Pick::Edit(index)) => {
                if let Some(path) = loaded.files.get(index).and_then(|file| file.path.clone()) {
                    self.edit_flow(&loaded.project, &path);
                    open = false;
                }
            }
            Some(Pick::New) => {
                let name = self.flows.new_name.clone();
                self.create_flow(&loaded, &name);
            }
            None => {}
        }
        self.flows.open = open && self.flows.open;
    }
}

/// Trust question for a project's flow files. Returns the clicked answer.
fn flow_trust_bar(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<bool> {
    if loaded.files.is_empty() || answer == Some(true) {
        return None;
    }
    let text = if answer == Some(false) {
        "This project is not trusted, so its flows stay off."
    } else {
        "Trust this project? A flow starts its commands and agents in your shell, with Enter."
    };
    settings_controls::description_line(ui, text);
    ui.horizontal(|ui| {
        if ui.button("Trust project").clicked() {
            Some(true)
        } else if answer.is_none() && ui.button("Don't trust").clicked() {
            Some(false)
        } else {
            None
        }
    })
    .inner
}

/// The second line of a flow row: the file and what it has.
fn file_line(file: &FlowFile, steps: usize) -> String {
    let name = file
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map_or_else(
            || file.file.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
    format!("{name}, {steps} steps")
}

/// One row per flow file with Run and Edit. Returns the clicked button.
fn flow_rows(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<Pick> {
    if loaded.files.is_empty() {
        settings_controls::description_line(
            ui,
            "No flows in this project. Make one below: it opens as a drawing, where a box is \
             a step, a diamond is a gate and an arrow is the order.",
        );
        return None;
    }
    let mut clicked = None;
    egui::Grid::new("run-flow-rows")
        .num_columns(3)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for (index, file) in loaded.files.iter().enumerate() {
                let allowed = file.flow.is_ok() && answer == Some(true);
                let why = if file.flow.is_err() {
                    "Correct the flow first"
                } else {
                    "Trust the project first"
                };
                let run = ui
                    .add_enabled(allowed, egui::Button::new("Run"))
                    .on_disabled_hover_text(why);
                if run.clicked() {
                    clicked = Some(Pick::Run(index));
                }
                let edit = ui
                    .add_enabled(file.path.is_some(), egui::Button::new("Edit"))
                    .on_disabled_hover_text("This name has two files; delete one of them");
                if edit.clicked() {
                    clicked = Some(Pick::Edit(index));
                }
                ui.vertical(|ui| match &file.flow {
                    Ok((name, steps)) => {
                        ui.strong(name);
                        ui.weak(file_line(file, *steps));
                    }
                    Err(error) => {
                        ui.strong(&file.file);
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
                ui.end_row();
            }
        });
    clicked
}

/// The name field and the New flow button.
fn new_flow_row(ui: &mut egui::Ui, name: &mut String) -> Option<Pick> {
    ui.separator();
    ui.horizontal(|ui| {
        let made = ui
            .button("New flow")
            .on_hover_text("Make a flow drawing from a template and open it")
            .clicked();
        let field = ui.add(egui::TextEdit::singleline(name).hint_text("Name, for example feature"));
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ((made || entered) && !name.trim().is_empty()).then_some(Pick::New)
    })
    .inner
}
