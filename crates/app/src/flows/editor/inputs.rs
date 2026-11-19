//! The flow's run inputs in the editor: one row per input (name,
//! description, default, required) in the inspector beside the rules. Save
//! keeps the flow file's strict check; two inputs with one name are an
//! error here, because the file has one table per name.
use super::FlowEditor;
use crate::appearance;
use std::collections::BTreeMap;
use terminator_core::flow::Input;

/// One input as the editor shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InputRow {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The default value; not used while `required`.
    pub(crate) default: String,
    /// No default: a run must set it.
    pub(crate) required: bool,
}

/// The editor rows of a flow's inputs, in name order.
pub(crate) fn rows_of(inputs: &BTreeMap<String, Input>) -> Vec<InputRow> {
    inputs
        .iter()
        .map(|(name, input)| InputRow {
            name: name.clone(),
            description: input.description.clone().unwrap_or_default(),
            default: input.default.clone().unwrap_or_default(),
            required: input.required(),
        })
        .collect()
}

/// The file's inputs of the editor rows.
pub(crate) fn inputs_of(rows: &[InputRow]) -> BTreeMap<String, Input> {
    rows.iter()
        .map(|row| {
            let description = row.description.trim();
            let input = Input {
                description: (!description.is_empty()).then(|| description.to_owned()),
                default: (!row.required).then(|| row.default.clone()),
            };
            (row.name.trim().to_owned(), input)
        })
        .collect()
}

/// The first input name that two rows use.
pub(crate) fn duplicate(rows: &[InputRow]) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .map(|row| row.name.trim())
        .find(|name| !seen.insert(*name))
        .map(|name| format!("Input \"{name}\" is there two times"))
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
}

/// The Inputs part of the inspector. `error` is the check error of the
/// inputs, if any.
pub(super) fn inputs_section(ui: &mut egui::Ui, editor: &mut FlowEditor, error: Option<&str>) {
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new("Run inputs")
            .size(appearance::FONT_HEADING)
            .strong(),
    );
    ui.weak(
        "Run asks for these values. Use {{inputs.NAME}} in a command, prompt, gate or the rules.",
    );
    ui.add_space(6.0);
    let mut remove = None;
    for (index, row) in editor.inputs.iter_mut().enumerate() {
        ui.push_id(("flow-input", index), |ui| {
            ui.horizontal(|ui| {
                caption(ui, "Name");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if appearance::sidebar_action(ui, "Trash", "Remove this input").clicked() {
                        remove = Some(index);
                    }
                });
            });
            ui.add(
                appearance::singleline(&mut row.name)
                    .desired_width(f32::INFINITY)
                    .hint_text("a-z, 0-9, - or _"),
            );
            caption(ui, "Description");
            ui.add(
                appearance::singleline(&mut row.description)
                    .desired_width(f32::INFINITY)
                    .hint_text("Optional"),
            );
            ui.checkbox(&mut row.required, "Required (no default)");
            ui.add_enabled_ui(!row.required, |ui| {
                ui.add(
                    appearance::singleline(&mut row.default)
                        .desired_width(f32::INFINITY)
                        .hint_text("Default value"),
                );
            });
            ui.add_space(8.0);
        });
    }
    if let Some(index) = remove {
        editor.inputs.remove(index);
    }
    let problem = duplicate(&editor.inputs);
    if let Some(text) = problem.as_deref().or(error) {
        let color = ui.visuals().error_fg_color;
        ui.label(
            egui::RichText::new(text)
                .size(appearance::FONT_CAPTION)
                .color(color),
        );
    }
    if ui.button("Add input").clicked() {
        let number = editor.inputs.len().saturating_add(1);
        editor.inputs.push(InputRow {
            name: format!("input{number}"),
            required: true,
            ..InputRow::default()
        });
    }
}
