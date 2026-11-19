//! The inspector: a panel at the right of the flow editor tab that edits
//! the selected card (ID, command, prompt, worktree, folder, and the steps
//! it runs after), or the flow's rules when no card is selected. It is not
//! modal: the canvas stays usable beside it. The first check error of the
//! flow shows under the field it concerns.
use super::FlowEditor;
use super::model::{Node, input, output, would_cycle};
use super::style::Colors;
use crate::appearance;
use crate::flows::status::{status_icon, step_style};
use egui_snarl::NodeId;
use std::collections::BTreeMap;
use terminator_core::flow::{Draft, FlowError, StepRun};

/// Width of the inspector panel.
pub(super) const WIDTH: f32 = 300.0;
const PROMPT_ROWS: usize = 8;
const RULE_ROWS: usize = 10;

/// The field that a check error concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Name,
    Rules,
    Id,
    Command,
    Prompt,
    Message,
    Folder,
    Worktree,
    RunsAfter,
    /// The flow's run inputs.
    Inputs,
    /// The whole step or the whole flow.
    Other,
}

/// The first check error of a flow: its step, its field and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckError {
    pub(crate) step: Option<String>,
    pub(crate) field: Field,
    pub(crate) text: String,
}

/// The check error of `draft`, with the step and the field it concerns.
pub(crate) fn check_error(draft: &Draft) -> Option<CheckError> {
    let error = draft.check().err()?;
    let (step, field) = error_place(&error);
    Some(CheckError {
        step,
        field,
        text: error.to_string(),
    })
}

/// The step and the field that a flow error concerns.
pub(crate) fn error_place(error: &FlowError) -> (Option<String>, Field) {
    let at = |step: &str, field| (Some(step.to_owned()), field);
    match error {
        FlowError::Name => (None, Field::Name),
        FlowError::Rules => (None, Field::Rules),
        FlowError::BadId(step) | FlowError::DuplicateId(step) => at(step, Field::Id),
        FlowError::Text(step, "command") => at(step, Field::Command),
        FlowError::Text(step, "prompt") => at(step, Field::Prompt),
        FlowError::Text(step, "gate") => at(step, Field::Message),
        FlowError::Cwd(step) => at(step, Field::Folder),
        FlowError::Worktree(step) | FlowError::DuplicateWorktree { step, .. } => {
            at(step, Field::Worktree)
        }
        FlowError::UnknownNeed { step, .. } | FlowError::Cycle(step) => at(step, Field::RunsAfter),
        FlowError::Kind(step) | FlowError::GateField(step) | FlowError::Text(step, _) => {
            at(step, Field::Other)
        }
        FlowError::BadInput(_) | FlowError::InputText(..) | FlowError::TooManyInputs => {
            (None, Field::Inputs)
        }
        FlowError::Placeholder { step, field, .. } => {
            let field = match *field {
                "command" => Field::Command,
                "prompt" => Field::Prompt,
                "gate" => Field::Message,
                "rules" => Field::Rules,
                _ => Field::Other,
            };
            (step.clone(), field)
        }
        _ => (None, Field::Other),
    }
}

/// What the inspector shows with each card.
pub(super) struct Show<'a> {
    pub(super) colors: Colors,
    pub(super) run: Option<&'a BTreeMap<String, StepRun>>,
    pub(super) error: Option<&'a CheckError>,
}

/// A labeled text field on the panel width, and the error of `field`.
fn field(
    ui: &mut egui::Ui,
    (label, hint, rows): (&str, &str, usize),
    text: &mut String,
    error: Option<&str>,
) {
    ui.label(
        egui::RichText::new(label)
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    let edit = if rows > 1 {
        egui::TextEdit::multiline(text).desired_rows(rows)
    } else {
        appearance::singleline(text)
    };
    ui.add(edit.desired_width(f32::INFINITY).hint_text(hint));
    error_line(ui, error);
    ui.add_space(6.0);
}

fn error_line(ui: &mut egui::Ui, error: Option<&str>) {
    if let Some(error) = error {
        let color = ui.visuals().error_fg_color;
        ui.label(
            egui::RichText::new(error)
                .size(appearance::FONT_CAPTION)
                .color(color),
        );
    }
}

/// The inspector of `editor`. Gives the session of a step whose terminal
/// the user asked for.
pub(super) fn show(ui: &mut egui::Ui, editor: &mut FlowEditor, show: &Show<'_>) -> Option<String> {
    let mut go = None;
    egui::ScrollArea::vertical()
        .id_salt("flow-inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match editor
                .selected
                .filter(|node| editor.snarl.get_node(*node).is_some())
            {
                Some(node) => go = node_fields(ui, editor, node, show),
                None => rules(ui, editor, show.error),
            }
        });
    go
}

/// The error text when it concerns `field` of step `id` (`None`: the
/// flow).
fn error_of<'a>(show: &Show<'a>, id: Option<&str>, field: Field) -> Option<&'a str> {
    show.error
        .filter(|error| error.field == field && error.step.as_deref() == id)
        .map(|error| error.text.as_str())
}

/// The flow's rules: the inspector when no card is selected.
fn rules(ui: &mut egui::Ui, editor: &mut FlowEditor, error: Option<&CheckError>) {
    ui.label(
        egui::RichText::new("Flow rules")
            .size(appearance::FONT_HEADING)
            .strong(),
    );
    ui.weak("Every agent of this flow gets this text before its prompt.");
    ui.add_space(8.0);
    let hint = "For example: Obey AGENTS.md. Report the result with terminator-hook flow report.";
    let concerns = |field| {
        error
            .filter(|error| error.field == field && error.step.is_none())
            .map(|error| error.text.as_str())
    };
    field(
        ui,
        ("Rules", hint, RULE_ROWS),
        &mut editor.rules,
        concerns(Field::Rules),
    );
    error_line(ui, concerns(Field::Name));
    super::inputs::inputs_section(ui, editor, concerns(Field::Inputs));
    ui.add_space(8.0);
    ui.weak("Select a card to edit its step. An agent with no prompt, or with its prompt in the command, does not get the rules.");
}

/// All fields of one card, the steps it runs after, and its run state.
fn node_fields(
    ui: &mut egui::Ui,
    editor: &mut FlowEditor,
    node: NodeId,
    show: &Show<'_>,
) -> Option<String> {
    let id = editor.snarl.get_node(node)?.id().trim().to_owned();
    let gate = editor.snarl.get_node(node)?.is_gate();
    ui.horizontal(|ui| {
        let (icon, title, color) = if gate {
            ("ShieldQuestion", "Gate", show.colors.gate)
        } else {
            ("Terminal", "Step", show.colors.step)
        };
        let size = egui::Vec2::splat(appearance::TOOLBAR_ICON);
        ui.add(
            egui::Image::new(crate::icons::source(icon))
                .tint(color)
                .fit_to_exact_size(size),
        );
        ui.label(
            egui::RichText::new(title)
                .size(appearance::FONT_HEADING)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if appearance::sidebar_action(ui, "X", "Close: show the flow rules").clicked() {
                editor.selected = None;
            }
        });
    });
    ui.weak(if gate {
        "The run stops here until you approve."
    } else {
        "One new terminal with this command."
    });
    ui.add_space(8.0);
    let error = |field| error_of(show, Some(&id), field);
    let value = editor.snarl.get_node_mut(node)?;
    let id_error = error(Field::Id);
    field(ui, ("ID", "a-z, 0-9, - or _", 1), value.id_mut(), id_error);
    match value {
        Node::Step {
            command,
            prompt,
            cwd,
            worktree,
            ..
        } => {
            field(
                ui,
                ("Command", "For example claude", 1),
                command,
                error(Field::Command),
            );
            let hint = "What the agent does in this step. Sent after the agent starts.";
            field(
                ui,
                ("Prompt", hint, PROMPT_ROWS),
                prompt,
                error(Field::Prompt),
            );
            let hint = "New worktree name (optional)";
            field(ui, ("Worktree", hint, 1), worktree, error(Field::Worktree));
            let hint = "In the project (optional)";
            field(ui, ("Folder", hint, 1), cwd, error(Field::Folder));
        }
        Node::Gate { message, .. } => {
            let hint = "The question of this gate";
            field(ui, ("You approve", hint, 4), message, error(Field::Message));
        }
    }
    runs_after(ui, editor, node);
    error_line(ui, error(Field::RunsAfter).or(error(Field::Other)));
    let step = show.run.and_then(|run| run.get(&id))?;
    this_run(ui, step, show.colors)
}

/// "Runs after": one checkbox per other card. A choice that would close a
/// cycle is off, with the reason on hover.
pub(super) fn runs_after(ui: &mut egui::Ui, editor: &mut FlowEditor, node: NodeId) {
    ui.label(
        egui::RichText::new("Runs after")
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    let mut others: Vec<(NodeId, String)> = editor
        .snarl
        .node_ids()
        .filter(|(other, _)| *other != node)
        .map(|(other, value)| (other, value.id().trim().to_owned()))
        .collect();
    others.sort_by(|a, b| a.1.cmp(&b.1));
    if others.is_empty() {
        ui.weak("No other steps.");
    }
    let needs = editor.snarl.in_pin(input(node)).remotes;
    for (other, id) in others {
        let mut on = needs.iter().any(|from| from.node == other);
        let cycle = !on && would_cycle(&editor.snarl, other, node);
        let label = if id.is_empty() {
            "no ID".to_owned()
        } else {
            id
        };
        let response = ui.add_enabled(!cycle, egui::Checkbox::new(&mut on, label));
        let response = response.on_disabled_hover_text("This would make a cycle.");
        if response.changed() {
            if on {
                editor.snarl.connect(output(other), input(node));
            } else {
                editor.snarl.disconnect(output(other), input(node));
            }
        }
    }
    ui.add_space(6.0);
}

/// The state of the selected step in the shown run, and its terminal.
fn this_run(ui: &mut egui::Ui, step: &StepRun, colors: Colors) -> Option<String> {
    ui.separator();
    ui.label(
        egui::RichText::new("This run")
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    let style = step_style(step);
    let color = colors.of_style(style);
    ui.horizontal(|ui| {
        status_icon(ui, style, color);
        ui.label(egui::RichText::new(style.label).color(color));
    });
    for text in [&step.waits_for, &step.note].into_iter().flatten() {
        ui.weak(text);
    }
    let session = step.session.as_ref()?;
    ui.button("Open terminal")
        .on_hover_text("Go to the terminal of this step")
        .clicked()
        .then(|| session.clone())
}
