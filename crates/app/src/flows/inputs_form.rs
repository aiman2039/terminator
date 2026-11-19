//! The Run inputs form: a flow with `[inputs]` asks for their values in a
//! small window before the run starts. Run in the form is the explicit
//! click; the trust answer and the unsaved-editor check still apply. The
//! values go to `flow run --set NAME=VALUE`, each as one quoted shell word.
use super::LoadedFlows;
use crate::*;
use std::path::PathBuf;
use terminator_core::flow::{Input, inputs::value_ok};

/// One field of the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) required: bool,
    pub(crate) value: String,
}

/// Where Run came from: the Flows window or tool window (the run opens the
/// flow's tab), or the flow editor of this path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Window,
    Editor(PathBuf),
}

/// The open form. GUI memory only.
pub(crate) struct InputsForm {
    pub(crate) loaded: LoadedFlows,
    pub(crate) file: String,
    pub(crate) origin: Origin,
    pub(crate) fields: Vec<Field>,
}

impl InputsForm {
    pub(crate) fn new(
        loaded: LoadedFlows,
        file: &str,
        inputs: &[(String, Input)],
        origin: Origin,
    ) -> Self {
        let fields = inputs
            .iter()
            .map(|(name, input)| Field {
                name: name.clone(),
                description: input.description.clone(),
                required: input.required(),
                value: input.default.clone().unwrap_or_default(),
            })
            .collect();
        Self {
            loaded,
            file: file.to_owned(),
            origin,
            fields,
        }
    }

    /// Why Run is off: a required input that is empty, or a value with a
    /// control character.
    pub(crate) fn problem(&self) -> Option<String> {
        self.fields.iter().find_map(|field| {
            if field.required && field.value.trim().is_empty() {
                Some(format!("{} is required", field.name))
            } else if !value_ok(&field.value) {
                Some(format!("{} has a control character", field.name))
            } else {
                None
            }
        })
    }

    pub(crate) fn values(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .map(|field| (field.name.clone(), field.value.clone()))
            .collect()
    }
}

/// "issue=42 · branch=main": a run's input values on one line.
pub(crate) fn inputs_line(values: &std::collections::BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Width of the form's fields.
const WIDTH: f32 = 340.0;

impl App {
    /// A Run click. A flow with inputs opens the form; any other flow, or a
    /// run that the trust answer or an unsaved editor stops, goes on at
    /// once (the refusal shows there).
    pub(crate) fn request_flow_run(
        &mut self,
        loaded: &LoadedFlows,
        file: &str,
        (inputs, origin): (&[(String, Input)], Origin),
    ) {
        let trusted = self.lsp.trust_answer(&loaded.root) == Some(true);
        if inputs.is_empty() || !trusted {
            self.start_flow_run(loaded, (file, &[]), &origin);
            return;
        }
        if self.flow_run_waits_for_save(loaded, file) {
            return;
        }
        self.flows.inputs_form = Some(InputsForm::new(loaded.clone(), file, inputs, origin));
    }

    /// Starts the run of `file` with `values`, from the Flows windows or
    /// the editor. False when it did not start.
    fn start_flow_run(
        &mut self,
        loaded: &LoadedFlows,
        (file, values): (&str, &[(String, String)]),
        origin: &Origin,
    ) -> bool {
        match origin {
            Origin::Window => self.run_flow_in_editor_with(loaded, file, values),
            Origin::Editor(path) => self.run_flow_from_editor(path, loaded, (file, values)),
        }
    }

    /// The form window, at the click. Run starts the run; Cancel, Esc or
    /// the close button start nothing.
    pub(crate) fn flow_inputs_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.flows.inputs_form.take() else {
            return;
        };
        let mut open = true;
        let mut run = false;
        let mut cancel = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let title = "Run inputs";
        self.popups
            .window(ctx, title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .auto_sized()
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                (run, cancel) = inputs_body(ui, &mut form, cancel);
            });
        if run && form.problem().is_none() {
            let values = form.values();
            self.start_flow_run(&form.loaded, (&form.file, &values), &form.origin);
        } else if open && !cancel {
            self.flows.inputs_form = Some(form);
        }
    }
}

/// The fields and the footer. Gives (Run, Cancel).
fn inputs_body(ui: &mut egui::Ui, form: &mut InputsForm, cancel: bool) -> (bool, bool) {
    ui.label(
        egui::RichText::new(format!("{}.toml", form.file))
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    ui.add_space(6.0);
    let mut entered = false;
    for field in &mut form.fields {
        let mark = if field.required { " *" } else { "" };
        ui.label(
            egui::RichText::new(format!("{}{mark}", field.name))
                .size(appearance::FONT_SMALL)
                .strong(),
        );
        if let Some(text) = &field.description {
            ui.label(
                egui::RichText::new(text)
                    .size(appearance::FONT_CAPTION)
                    .weak(),
            );
        }
        let hint = if field.required {
            "Required"
        } else {
            "Optional"
        };
        let edit = ui.add(
            appearance::singleline(&mut field.value)
                .desired_width(f32::INFINITY)
                .hint_text(hint),
        );
        entered |= edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.add_space(6.0);
    }
    let problem = form.problem();
    let text = problem.clone().unwrap_or_else(|| {
        "Values are typed as plain text; in a command each value is one quoted word.".into()
    });
    ui.label(
        egui::RichText::new(text)
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    ui.add_space(8.0);
    let (mut run, mut stop) = (false, cancel);
    appearance::footer_row(ui, |ui| {
        ui.add_enabled_ui(problem.is_none(), |ui| {
            let button = appearance::primary_button(ui, "Run");
            let button = match &problem {
                Some(why) => button.on_disabled_hover_text(why),
                None => button.on_hover_text("Start the run with these values"),
            };
            run = button.clicked() || (entered && problem.is_none());
        });
        stop |= appearance::secondary_button(ui, "Cancel").clicked();
    });
    (run, stop)
}
