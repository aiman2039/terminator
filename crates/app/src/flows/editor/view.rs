//! Paints a flow editor tab: the toolbar, the graph on the whole pane, and
//! the editor's dialogs (one card's fields, the prompt rules, the preview).
use super::dialogs::{node_dialog, preview_dialog, rules_dialog};
use super::model::Node;
use super::viewer::{self, Colors, Viewer};
use super::{Dialog, FlowEditor, Slot, Stay};
use crate::app::{OpenFileInput, OpenFileModeInput};
use crate::document_view::DocumentKind;
use crate::flows::panel::run_status;
use crate::flows::{LoadedFlows, Run};
use crate::*;
use egui_snarl::ui::SnarlWidget;
use std::path::Path;
use terminator_core::flow::control;

/// One click in the editor's toolbar or in a dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Click {
    Save,
    Run,
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
            failed: appearance::color(&self.theme.status_failed),
        }
    }

    /// The project ID of an editor's flow.
    fn flow_project(&self, editor: &FlowEditor) -> Option<String> {
        let project = self
            .state
            .projects
            .iter()
            .find(|project| project.path == editor.root)?;
        Some(project.id.clone())
    }

    /// The newest run of the editor's saved flow: its folder and state.
    /// The cards show it, so the editor is also the view of a run.
    fn flow_run_of(&mut self, ctx: &egui::Context, editor: &FlowEditor) -> Option<Run> {
        if !editor.on_disk {
            return None;
        }
        self.poll_flow_runs(ctx);
        let project = self.flow_project(editor)?;
        let run = self
            .flows
            .runs
            .iter()
            .find(|run| run.state.project == project && run.state.flow == editor.saved.name)?;
        Some(Run {
            dir: run.dir.clone(),
            state: run.state.clone(),
        })
    }

    /// The graph, with the state of `run` on its cards. Does the clicks
    /// on a card's run row.
    fn flow_graph(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        (editor, run): (&mut FlowEditor, Option<&Run>),
    ) {
        let colors = self.flow_colors();
        let fit = std::mem::take(&mut editor.fit).then(|| ui.available_rect_before_wrap());
        let mut viewer = Viewer {
            colors,
            run: run.map(|run| &run.state.steps),
            go: None,
            answer: None,
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
        if let Some(session) = viewer.go.take() {
            self.go_session(&session);
        }
        if let (Some((step, approve)), Some(run)) = (viewer.answer.take(), run) {
            let dir = run.dir.clone();
            self.flow_act(move || control::answer_gate(&dir, &step, approve));
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
        let run = self.flow_run_of(ui.ctx(), editor);
        let state = run.as_ref().map(|run| run_status(&run.state));
        let mut click = toolbar(ui, editor, state);
        if let Some(error) = &editor.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        self.flow_graph(ui, path, (editor, run.as_ref()));
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
            Some(Click::Run) => {
                if let Some(project) = self.flow_project(editor) {
                    let loaded = LoadedFlows {
                        project,
                        root: editor.root.clone(),
                        files: Vec::new(),
                    };
                    if self.run_flow(&loaded, &editor.file) {
                        // The editor shows the run, so the app stays on it.
                        self.flows.stay = Some(Stay::after_run(path, &self.flows.runs));
                    }
                }
            }
            Some(Click::AddStep) => drop(viewer::add_node(&mut editor.snarl, place, false)),
            Some(Click::AddGate) => drop(viewer::add_node(&mut editor.snarl, place, true)),
            Some(Click::Fit) => editor.fit = true,
            Some(Click::Rules) => editor.dialog = Some(Dialog::Rules),
            Some(Click::Preview) => editor.dialog = Some(Dialog::Preview),
            Some(Click::CopyPlan) => {
                ctx.copy_text(editor.plan());
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

    /// The open dialog of the editor: a modal in the center of the window.
    /// It takes the input; a click outside it, Esc or Done closes it.
    fn flow_editor_dialog(
        &mut self,
        ctx: &egui::Context,
        editor: &mut FlowEditor,
    ) -> Option<Click> {
        let dialog = editor.dialog?;
        let (title, about) = match dialog {
            Dialog::Node(node) if editor.snarl.get_node(node).is_some_and(Node::is_gate) => {
                ("Edit gate", "The run stops here until you approve.")
            }
            Dialog::Node(_) => ("Edit step", "One new terminal with this command."),
            Dialog::Rules => ("Flow rules", "Global rules of this flow."),
            Dialog::Preview => ("Flow plan", "The flow as Markdown text."),
        };
        let width = DIALOG_WIDTH.min(ctx.content_rect().width() - 80.0);
        let surface = appearance::color(&self.theme.surface);
        let mut click = None;
        let mut done = false;
        let modal = egui::Modal::new(egui::Id::unique("flow-editor-dialog"))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(surface)
                    .corner_radius(appearance::RADIUS_LARGE)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.heading(title);
                ui.weak(about);
                ui.add_space(12.0);
                match dialog {
                    Dialog::Node(node) => done = !node_dialog(ui, editor, node),
                    Dialog::Rules => rules_dialog(ui, editor),
                    Dialog::Preview => click = preview_dialog(ui, editor),
                }
                ui.add_space(12.0);
                appearance::footer_row(ui, |ui| {
                    done |= appearance::primary_button(ui, "Done").clicked();
                });
            });
        if modal.should_close() || done {
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
fn toolbar(ui: &mut egui::Ui, editor: &mut FlowEditor, run: Option<&str>) -> Option<Click> {
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
        ui.add_enabled_ui(!dirty && run != Some("running"), |ui| {
            let tip = "Run the saved flow: one new terminal per step. The cards show the run.";
            set(tool(ui, "Play", "Run", tip), Click::Run);
        });
        set(
            tool(ui, "Settings2", "Rules", "Global rules of this flow"),
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
        let run = run.map(|run| format!(" · Run: {run}")).unwrap_or_default();
        let steps = editor.snarl.nodes().count();
        ui.weak(format!("{steps} steps · {state}{run}"));
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
