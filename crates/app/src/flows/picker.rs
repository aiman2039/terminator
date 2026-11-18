//! The Flows window: the project's flow files with Run and Edit, and New
//! flow. New flow and Edit open the native flow editor. Making or editing
//! a flow needs no trust answer; Run does.
use super::LoadedFlows;
use crate::app::OpenFileInput;
use crate::*;
use std::path::Path;

/// One click in the Flows window.
enum Pick {
    Run(usize),
    Edit(usize),
    New,
    Editor,
    Trust(bool),
}

impl App {
    /// Opens a flow file as text in the editor tab: for a file that is
    /// not a correct flow, so the flow editor cannot show it.
    pub(super) fn open_flow_as_text(&mut self, project: &str, path: &Path) {
        if !self.project_available(project) {
            self.info = Some("The flow's project is not open.".into());
            return;
        }
        if self.selected.as_deref() != Some(project) {
            self.select_project(project.to_owned());
        }
        self.hide_center_overlay();
        self.open_file(OpenFileInput {
            path: path.to_path_buf(),
            line: None,
            split: None,
            external: false,
        });
    }

    /// The Flows window.
    pub(crate) fn flow_dialog(&mut self, ctx: &egui::Context) {
        self.poll_flow_editor(ctx);
        #[cfg(feature = "test-support")]
        self.fixture_flow_editor();
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
        let editing = self
            .flows
            .editor
            .as_ref()
            .map(|editor| (editor.file().to_owned(), editor.dirty()));
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
                    .or(new_flow_row(ui, &mut name))
                    .or(editor_row(ui, editing.as_ref()));
            });
        self.flows.new_name = name;
        if self.flows.loading.is_some() {
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
                if let Some(file) = loaded.files.get(index) {
                    self.edit_flow_editor(&loaded.project, loaded.root.clone(), &file.file);
                    open = false;
                }
            }
            Some(Pick::New) => {
                let name = std::mem::take(&mut self.flows.new_name);
                self.new_flow_editor(loaded.root.clone(), &name);
                // A refused name stays in the field.
                if self.center == CenterPage::FlowEditor {
                    open = false;
                } else {
                    self.flows.new_name = name;
                }
            }
            Some(Pick::Editor) => {
                self.show_center(CenterPage::FlowEditor);
                open = false;
            }
            None => {}
        }
        self.flows.open = open;
    }

    /// Test builds only: `TERMINATOR_FIXTURE_FLOW_EDITOR=NAME` opens the
    /// editor with flow NAME (or a new flow) of the selected project one
    /// time, so a fixture can look at the page without a click.
    #[cfg(feature = "test-support")]
    fn fixture_flow_editor(&mut self) {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let Ok(name) = std::env::var("TERMINATOR_FIXTURE_FLOW_EDITOR") else {
            return;
        };
        let Some(project) = self.selected_project() else {
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        if DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let there =
            terminator_core::flow::files::path_of(&root, &name).is_ok_and(|path| path.is_file());
        if there {
            self.edit_flow_editor(&id, root, &name);
        } else {
            self.new_flow_editor(root, &name);
        }
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

/// One row per flow file with Run and Edit. Returns the clicked button.
fn flow_rows(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<Pick> {
    if loaded.files.is_empty() {
        settings_controls::description_line(
            ui,
            "No flows in this project. Make one below: it opens in the flow editor, where \
             you add steps and gates and connect them in the order that they run.",
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
                if ui.button("Edit").clicked() {
                    clicked = Some(Pick::Edit(index));
                }
                ui.vertical(|ui| match &file.flow {
                    Ok((name, steps)) => {
                        ui.strong(name);
                        ui.weak(format!("{}.toml, {steps} steps", file.file));
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
            .on_hover_text("Open the flow editor with a new flow")
            .clicked();
        let field = ui.add(egui::TextEdit::singleline(name).hint_text("Name, for example feature"));
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ((made || entered) && !name.trim().is_empty()).then_some(Pick::New)
    })
    .inner
}

/// The flow that is open in the editor, with a button to go back to it.
fn editor_row(ui: &mut egui::Ui, editing: Option<&(String, bool)>) -> Option<Pick> {
    let (file, dirty) = editing?;
    ui.separator();
    ui.horizontal(|ui| {
        let open = ui.button("Open editor").clicked();
        ui.label(if *dirty {
            format!("{file}: changes that are not saved")
        } else {
            format!("{file} is open in the editor")
        });
        open.then_some(Pick::Editor)
    })
    .inner
}
