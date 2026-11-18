//! Paints a flow editor tab: the toolbar, the graph on the whole pane, and
//! the editor's dialogs (one card's fields, the prompt rules, the preview).
use super::model::{self, Node};
use super::viewer::{self, Colors, Viewer};
use super::{Dialog, FlowEditor, Slot};
use crate::app::{OpenFileInput, OpenFileModeInput};
use crate::document_view::DocumentKind;
use crate::*;
use egui_snarl::{NodeId, ui::SnarlWidget};
use std::path::Path;

/// One click in the editor's toolbar or in a dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Click {
    Save,
    AddStep,
    AddGate,
    Fit,
    Float,
    Rules,
    Preview,
    CopyPlan,
    CopyFile,
    ExportPlan,
    Discard,
}

/// Rows of the large text fields.
const PROMPT_ROWS: usize = 14;
const RULE_ROWS: usize = 6;
const DIALOG_WIDTH: f32 = 620.0;

impl App {
    /// The flow editor of the flow file `path`, on the whole pane.
    pub(crate) fn flow_editor_view(&mut self, ui: &mut egui::Ui, path: &Path) {
        self.flows.mark_visible(path);
        let slot = self
            .flows
            .editors
            .remove(path)
            .unwrap_or_else(|| Slot::load(path))
            .poll(path);
        let keep = match slot {
            Slot::Loading(receiver) => {
                ui.spinner();
                ui.ctx().request_repaint();
                Some(Slot::Loading(receiver))
            }
            Slot::Failed(error) => self.flow_failed_view(ui, path, error),
            Slot::Ready(mut editor) => self
                .flow_ready_view(ui, path, &mut editor)
                .then(|| Slot::Ready(editor)),
        };
        if let Some(slot) = keep {
            self.flows.editors.insert(path.into(), slot);
        }
    }

    /// A file that is not a correct flow: its error, and the way to repair
    /// it as text.
    fn flow_failed_view(&mut self, ui: &mut egui::Ui, path: &Path, error: String) -> Option<Slot> {
        ui.add_space(12.0);
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("This flow has an error: {error}"),
        );
        let mut again = false;
        ui.horizontal(|ui| {
            if ui.button("Open as text").clicked() {
                self.open_file_mode(OpenFileModeInput {
                    path: path.into(),
                    line: None,
                    split: None,
                    external: false,
                    text: true,
                });
            }
            again = ui.button("Read again").clicked();
        });
        (!again).then_some(Slot::Failed(error))
    }

    fn flow_colors(&self) -> Colors {
        Colors {
            canvas: appearance::color(&self.theme.window),
            node: appearance::color(&self.theme.surface),
            border: appearance::color(&self.theme.border),
            step: appearance::color(&self.theme.status_running),
            gate: appearance::color(&self.theme.status_waiting),
        }
    }

    /// The editor of a read flow. False when the user discarded it: the
    /// next frame reads the file again.
    fn flow_ready_view(&mut self, ui: &mut egui::Ui, path: &Path, editor: &mut FlowEditor) -> bool {
        if let Some(note) = editor.poll() {
            self.info = Some(note);
        }
        if let Some(plan) = editor.poll_export() {
            self.info = Some(format!(
                "Saved the plan to {}.",
                services::compact_path(&plan)
            ));
            self.open_file(OpenFileInput {
                path: plan,
                line: None,
                split: None,
                external: false,
            });
        }
        if editor.saving.is_some() || editor.exporting.is_some() {
            ui.ctx().request_repaint();
        }
        let colors = self.flow_colors();
        let mut click = toolbar(ui, editor);
        if let Some(error) = &editor.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let fit = std::mem::take(&mut editor.fit).then(|| ui.available_rect_before_wrap());
        let mut viewer = Viewer {
            colors,
            fit,
            edit: None,
            refit: false,
        };
        SnarlWidget::new()
            .id_salt(("flow-editor", path))
            .style(viewer::style(colors))
            .show(&mut editor.snarl, &mut viewer, ui);
        if let Some(node) = viewer.edit {
            editor.dialog = Some(Dialog::Node(node));
        }
        editor.fit |= viewer.refit;
        click = click.or(self.flow_editor_dialog(ui.ctx(), editor));
        self.flow_editor_click((ui.ctx(), path), editor, click)
    }

    /// Does one click. False for Discard.
    fn flow_editor_click(
        &mut self,
        (ctx, path): (&egui::Context, &Path),
        editor: &mut FlowEditor,
        click: Option<Click>,
    ) -> bool {
        let place = next_place(editor);
        match click {
            Some(Click::Save) => editor.save(),
            Some(Click::AddStep) => drop(viewer::add_node(&mut editor.snarl, place, false)),
            Some(Click::AddGate) => drop(viewer::add_node(&mut editor.snarl, place, true)),
            Some(Click::Fit) => editor.fit = true,
            Some(Click::Rules) => editor.dialog = Some(Dialog::Rules),
            Some(Click::Preview) => editor.dialog = Some(Dialog::Preview),
            Some(Click::CopyPlan) => {
                ctx.copy_text(model::plan_text(&editor.draft()));
                self.info = Some("Copied the plan.".into());
            }
            Some(Click::CopyFile) => match editor.draft().to_toml() {
                Ok(text) => {
                    ctx.copy_text(text);
                    self.info = Some("Copied the flow file text.".into());
                }
                Err(error) => editor.error = Some(error.to_string()),
            },
            Some(Click::ExportPlan) => editor.export_plan(),
            Some(Click::Float) => self.queue_dock_op(DockOp::Float {
                pane: Tab::Document {
                    path: path.into(),
                    kind: DocumentKind::Flow,
                },
                source: None,
            }),
            Some(Click::Discard) => return false,
            None => {}
        }
        true
    }

    /// The open dialog of the editor, as a window.
    fn flow_editor_dialog(
        &mut self,
        ctx: &egui::Context,
        editor: &mut FlowEditor,
    ) -> Option<Click> {
        let dialog = editor.dialog?;
        let title = match dialog {
            Dialog::Node(_) => "Edit step",
            Dialog::Rules => "Prompt rules",
            Dialog::Preview => "Flow preview",
        };
        let mut open = true;
        let mut click = None;
        let mut done = false;
        self.popups
            .window(ctx, title)
            .open(&mut open)
            .collapsible(false)
            .default_width(DIALOG_WIDTH)
            .show(ctx, |ui| {
                ui.set_min_width(DIALOG_WIDTH);
                match dialog {
                    Dialog::Node(node) => done = !node_dialog(ui, editor, node),
                    Dialog::Rules => rules_dialog(ui, editor),
                    Dialog::Preview => click = preview_dialog(ui, editor),
                }
                ui.add_space(6.0);
                appearance::footer_row(ui, |ui| done |= ui.button("Done").clicked());
            });
        if !open || done {
            editor.dialog = None;
        }
        click
    }
}

/// A toolbar button with an icon and a text.
fn tool(ui: &mut egui::Ui, icon: &str, label: &str, tip: &str) -> bool {
    appearance::icon_label_button(appearance::IconLabelButtonInput {
        ui,
        icon,
        label,
        height: appearance::TOOLBAR_BUTTON,
    })
    .on_hover_text(tip)
    .clicked()
}

/// The toolbar: the flow's name, add, Save, Rules, Export, the view
/// buttons and the state of the flow.
fn toolbar(ui: &mut egui::Ui, editor: &mut FlowEditor) -> Option<Click> {
    let mut click = None;
    let mut set = |on: bool, value: Click| {
        if on {
            click = Some(value);
        }
    };
    let (dirty, saving) = (editor.dirty(), editor.saving.is_some());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add(
            appearance::singleline(&mut editor.title)
                .desired_width(190.0)
                .hint_text("Flow name"),
        )
        .on_hover_text(format!(
            "The name of the flow. Its file is {}.toml",
            editor.file
        ));
        ui.separator();
        set(
            tool(
                ui,
                "Plus",
                "Step",
                "Add a step: a command in a new terminal",
            ),
            Click::AddStep,
        );
        set(
            tool(
                ui,
                "Plus",
                "Gate",
                "Add a gate: the run waits for your approval",
            ),
            Click::AddGate,
        );
        ui.separator();
        ui.add_enabled_ui(dirty && !saving, |ui| {
            set(
                tool(ui, "Save", "Save", "Check the flow and write its file"),
                Click::Save,
            );
        });
        set(
            tool(
                ui,
                "Settings2",
                "Rules",
                "Text before and after the prompt of each step",
            ),
            Click::Rules,
        );
        set(
            tool(ui, "Eye", "Preview", "The plan as text"),
            Click::Preview,
        );
        if let Some(value) = view_tools(ui, editor, dirty) {
            set(true, value);
        }
        let state = if saving {
            "Saving…"
        } else if !editor.on_disk {
            "New flow, not saved"
        } else if dirty {
            "Changes not saved"
        } else {
            "Saved"
        };
        ui.weak(format!("{} steps · {state}", editor.snarl.nodes().count()));
    });
    ui.add_space(4.0);
    click
}

/// The right part of the toolbar: Export, Fit to view, Float window and
/// the "more" menu.
fn view_tools(ui: &mut egui::Ui, editor: &FlowEditor, dirty: bool) -> Option<Click> {
    let mut click = None;
    let mut set = |on: bool, value: Click| {
        if on {
            click = Some(value);
        }
    };
    appearance::text_menu_button(ui, "Export", |ui| {
        set(
            appearance::menu_action(ui, true, "Copy plan as Markdown", "Copy").clicked(),
            Click::CopyPlan,
        );
        set(
            appearance::menu_action(ui, true, "Copy flow file text", "Copy").clicked(),
            Click::CopyFile,
        );
        let label = format!("Save plan as {}.md", editor.file);
        set(
            appearance::menu_action(ui, true, &label, "Download").clicked(),
            Click::ExportPlan,
        );
    });
    ui.separator();
    set(
        appearance::sidebar_action(ui, "Maximize", "Fit to view").clicked(),
        Click::Fit,
    );
    set(
        appearance::sidebar_action(ui, "ExternalLink", "Float window").clicked(),
        Click::Float,
    );
    appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
        let discard = appearance::menu_danger_action(ui, dirty, "Discard changes", "Trash");
        set(discard.clicked(), Click::Discard);
    });
    click
}

/// Where a node from the toolbar goes: below the lowest node.
fn next_place(editor: &FlowEditor) -> egui::Pos2 {
    let lowest = editor
        .snarl
        .nodes_pos()
        .map(|(position, _)| position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    if lowest.is_finite() {
        egui::pos2(0.0, lowest + 280.0)
    } else {
        egui::Pos2::ZERO
    }
}

/// A labeled text field on the whole dialog width.
fn field(ui: &mut egui::Ui, (label, hint): (&str, &str), text: &mut String, rows: usize) {
    ui.label(egui::RichText::new(label).strong());
    let edit = if rows > 1 {
        egui::TextEdit::multiline(text).desired_rows(rows)
    } else {
        egui::TextEdit::singleline(text)
    };
    ui.add(edit.desired_width(f32::INFINITY).hint_text(hint));
    ui.add_space(4.0);
}

/// All fields of one card, large. False when the card is gone.
fn node_dialog(ui: &mut egui::Ui, editor: &mut FlowEditor, node: NodeId) -> bool {
    let Some(value) = editor.snarl.get_node_mut(node) else {
        return false;
    };
    field(ui, ("ID", "a-z, 0-9, - or _"), value.id_mut(), 1);
    egui::ScrollArea::vertical()
        .max_height(420.0)
        .show(ui, |ui| match value {
            Node::Step {
                command,
                prompt,
                cwd,
                worktree,
                ..
            } => {
                field(ui, ("Command", "For example claude"), command, 1);
                let hint = "What the agent does in this step. Sent after the agent starts.";
                field(ui, ("Prompt", hint), prompt, PROMPT_ROWS);
                field(ui, ("Folder", "In the project (optional)"), cwd, 1);
                let hint = "Name of a new worktree for this step (optional)";
                field(ui, ("Worktree", hint), worktree, 1);
            }
            Node::Gate { message, .. } => {
                field(
                    ui,
                    ("You approve", "The question of this gate"),
                    message,
                    RULE_ROWS,
                );
            }
        });
    true
}

/// The rules text of the flow.
fn rules_dialog(ui: &mut egui::Ui, editor: &mut FlowEditor) {
    ui.weak(
        "This text goes before and after the prompt of each step when the flow runs. \
         A step with no prompt, or with its prompt in the command, does not get it.",
    );
    ui.add_space(6.0);
    field(
        ui,
        ("Before each prompt", "For example: Obey AGENTS.md."),
        &mut editor.prefix,
        RULE_ROWS,
    );
    let hint = "For example: Report the result with terminator-hook flow report.";
    field(
        ui,
        ("After each prompt", hint),
        &mut editor.suffix,
        RULE_ROWS,
    );
}

/// The plan as Markdown, read-only, with the copy and export buttons.
fn preview_dialog(ui: &mut egui::Ui, editor: &FlowEditor) -> Option<Click> {
    let draft = editor.draft();
    if let Err(error) = draft.clone().check() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Not a correct flow yet: {error}"),
        );
    }
    let plan = model::plan_text(&draft);
    egui::ScrollArea::vertical()
        .max_height(420.0)
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
