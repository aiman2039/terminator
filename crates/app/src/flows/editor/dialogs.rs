//! The contents of the flow editor's dialogs: one card's fields, the flow
//! rules, and the plan preview.
use super::FlowEditor;
use super::model::Node;
use super::view::Click;
use crate::*;
use egui_snarl::NodeId;

/// Rows of the large text fields.
const PROMPT_ROWS: usize = 14;
const RULE_ROWS: usize = 6;

/// A labeled text field on the whole dialog width.
fn field(ui: &mut egui::Ui, (label, hint): (&str, &str), text: &mut String, rows: usize) {
    ui.label(egui::RichText::new(label).small().weak());
    let edit = if rows > 1 {
        egui::TextEdit::multiline(text).desired_rows(rows)
    } else {
        egui::TextEdit::singleline(text)
    };
    ui.add(edit.desired_width(f32::INFINITY).hint_text(hint));
    ui.add_space(8.0);
}

/// All fields of one card, large. False when the card is gone.
pub(super) fn node_dialog(ui: &mut egui::Ui, editor: &mut FlowEditor, node: NodeId) -> bool {
    let Some(value) = editor.snarl.get_node_mut(node) else {
        return false;
    };
    match value {
        Node::Step {
            id,
            command,
            prompt,
            cwd,
            worktree,
        } => {
            ui.columns(2, |columns| {
                if let [left, right] = columns {
                    field(left, ("ID", "a-z, 0-9, - or _"), id, 1);
                    field(right, ("Command", "For example claude"), command, 1);
                }
            });
            let hint = "What the agent does in this step. Sent after the agent starts.";
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| field(ui, ("Prompt", hint), prompt, PROMPT_ROWS));
            ui.columns(2, |columns| {
                if let [left, right] = columns {
                    field(left, ("Folder", "In the project (optional)"), cwd, 1);
                    let hint = "New worktree name (optional)";
                    field(right, ("Worktree", hint), worktree, 1);
                }
            });
        }
        Node::Gate { id, message } => {
            field(ui, ("ID", "a-z, 0-9, - or _"), id, 1);
            let hint = "The question of this gate";
            field(ui, ("You approve", hint), message, RULE_ROWS);
        }
    }
    true
}

/// The rules text of the flow: one text for all steps.
pub(super) fn rules_dialog(ui: &mut egui::Ui, editor: &mut FlowEditor) {
    let hint = "For example: Obey AGENTS.md. Report the result with terminator-hook flow report.";
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            let label = ("Rules for every step", hint);
            field(ui, label, &mut editor.rules, PROMPT_ROWS);
        });
    ui.weak(
        "Each step's agent gets this text before its prompt. A step with no prompt, \
         or with its prompt in the command, does not get it.",
    );
}

/// The plan as Markdown, read-only, with the copy and export buttons.
pub(super) fn preview_dialog(ui: &mut egui::Ui, editor: &FlowEditor) -> Option<Click> {
    let draft = editor.draft();
    if let Err(error) = draft.clone().check() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Not a correct flow yet: {error}"),
        );
    }
    let plan = editor.plan();
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut plan.as_str())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = format!("Save as {}.md", editor.file);
        if ui.button("Copy plan").clicked() {
            Some(Click::CopyPlan)
        } else if ui.button("Copy flow file text").clicked() {
            Some(Click::CopyFile)
        } else if ui.button(label).clicked() {
            Some(Click::ExportPlan)
        } else {
            None
        }
    })
    .inner
}
