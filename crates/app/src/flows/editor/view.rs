//! Paints a flow editor tab: the toolbar, the run strip, the graph, and
//! the inspector at the right of the graph. The preview of the plan is a
//! dialog.
use super::banner::{StripClick, run_strip, starting_strip, still_starting};
use super::inspector::{self, CheckError, check_error};
use super::style::Colors;
use super::toolbar::{Click, Keys, toolbar};
use super::viewer::{self, Viewer};
use super::{Dialog, FlowEditor, Slot, canvas, model};
use crate::app::OpenFileInput;
use crate::document_view::DocumentKind;
use crate::flows::panel::Act;
use crate::flows::{LoadedFlows, Run};
use crate::*;
use egui_snarl::ui::SnarlWidget;
use std::collections::HashMap;
use std::path::Path;

/// Narrower panes show the inspector only for a selected card or the
/// rules.
const SPLIT_WIDTH: f32 = 1000.0;

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
            Slot::Failed(failure) => self.flow_failed_view(ui, path, failure),
            Slot::Ready(mut editor) => self
                .flow_ready_view(ui, path, &mut editor)
                .then(|| Slot::Ready(editor)),
        };
        if let Some(slot) = keep {
            self.flows.editors.insert(path.into(), slot);
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

    /// The newest run of the editor's flow file: its folder and state.
    /// The cards show it, so the editor is also the view of a run.
    fn flow_run_of(&mut self, ctx: &egui::Context, editor: &FlowEditor) -> Option<Run> {
        if !editor.on_disk {
            return None;
        }
        self.poll_flow_runs(ctx);
        let project = self.flow_project(editor)?;
        self.flows
            .runs
            .iter()
            .find(|run| run.of_flow(&project, &editor.file))
            .cloned()
    }

    /// The typed answers of `run`'s questions, by step ID.
    fn run_drafts(&self, run: Option<&Run>) -> HashMap<String, String> {
        let Some(run) = run else {
            return HashMap::new();
        };
        self.flows
            .drafts
            .iter()
            .filter(|((dir, _), _)| *dir == run.dir)
            .map(|((_, step), text)| (step.clone(), text.clone()))
            .collect()
    }

    /// The typed mailbox messages of `run`'s running steps, by step ID.
    fn run_mail_drafts(&self, run: Option<&Run>) -> HashMap<String, String> {
        let Some(run) = run else {
            return HashMap::new();
        };
        self.flows
            .mail_drafts
            .iter()
            .filter(|((dir, _), _)| *dir == run.dir)
            .map(|((_, step), text)| (step.clone(), text.clone()))
            .collect()
    }

    /// The graph, with the state of `run` on its cards. Does the clicks
    /// on the cards. `inspector` is the width of the inspector at the
    /// right of the canvas (0 when it is hidden).
    fn flow_graph(
        &mut self,
        (ui, inspector): (&mut egui::Ui, f32),
        path: &Path,
        (editor, run, error): (&mut FlowEditor, Option<&Run>, Option<&CheckError>),
    ) {
        let colors = Colors::of(&self.theme);
        let canvas = ui.available_rect_before_wrap();
        // A new canvas size (a resized pane or window, or the inspector
        // that opens or closes) fits the view again.
        if editor.canvas.is_some_and(|size| {
            (size.x - canvas.width()).abs() > 1.0 || (size.y - canvas.height()).abs() > 1.0
        }) {
            editor.fit = true;
        }
        editor.canvas = Some(canvas.size());
        let fit = editor.fit.then_some((canvas, inspector));
        // The first fit uses estimated sizes: fit again once all are known.
        editor.fit &= !canvas::measured(&editor.snarl, &editor.sizes);
        let mut drafts = self.run_drafts(run);
        let mut mail_drafts = self.run_mail_drafts(run);
        let (now, minutes) = (terminator_core::now(), self.preferences.flow_stall_minutes);
        let stalls = run
            .map(|run| &run.state.steps)
            .into_iter()
            .flatten()
            .filter_map(|(id, step)| {
                let stall = crate::flows::status::stall(step, &self.state, now, minutes)?;
                Some((id.clone(), stall))
            })
            .collect();
        let updates = run
            .map(|run| &run.state.steps)
            .into_iter()
            .flatten()
            .filter_map(|(id, step)| {
                let sid = step.session.as_deref()?;
                let preview = self.present_session(sid).notice_preview.clone()?;
                Some((id.clone(), preview))
            })
            .collect();
        let mut viewer = Viewer {
            colors,
            run: run.map(|run| &run.state.steps),
            live: run.is_some_and(|run| !run.ended()),
            stalls,
            updates,
            drafts: &mut drafts,
            mail_drafts: &mut mail_drafts,
            selected: editor.selected,
            error,
            sizes: &mut editor.sizes,
            fit,
            out: viewer::Out::default(),
        };
        let widget = SnarlWidget::new()
            .id_salt(("flow-editor", path))
            .style(viewer::style(colors));
        let before = viewer.sizes.clone();
        widget.show(&mut editor.snarl, &mut viewer, ui);
        let out = viewer.out;
        grown_apart(editor, &before, ui.ctx());
        editor.marked = widget.get_selected_nodes(ui);
        if let Some(run) = run {
            for (step, text) in drafts {
                self.flows.drafts.insert((run.dir.clone(), step), text);
            }
            for (step, text) in mail_drafts {
                self.flows.mail_drafts.insert((run.dir.clone(), step), text);
            }
        }
        self.flow_canvas_out(ui, (editor, run, canvas), out);
    }

    /// Takes the clicks of the canvas: selection, the run's actions, Fit
    /// and Tidy layout.
    fn flow_canvas_out(
        &mut self,
        ui: &egui::Ui,
        (editor, run, canvas): (&mut FlowEditor, Option<&Run>, egui::Rect),
        out: viewer::Out,
    ) {
        if let Some(node) = out.select {
            editor.selected = Some(node);
            editor.rules_open = false;
        } else if viewer::clicked_in(ui, canvas) {
            editor.selected = None;
            editor.rules_open = false;
        }
        if editor.fit {
            ui.ctx().request_repaint();
        }
        editor.fit |= out.refit;
        if out.tidy {
            tidy(editor);
        }
        if let Some(session) = out.go {
            self.go_session(&session);
        }
        if let Some(session) = out.peek {
            self.open_session_peek(&session);
        }
        if let Some(path) = out.open_diff {
            self.apply_flow_act(Act::OpenDiff(path));
        }
        let Some(run) = run else {
            return;
        };
        if let Some((step, choice)) = out.answer {
            self.apply_flow_act(Act::Gate(run.dir.clone(), step, choice));
        }
        if let Some(step) = out.send {
            self.apply_flow_act(Act::Answer(run.dir.clone(), step));
        }
        if let Some(step) = out.send_mail {
            self.apply_flow_act(Act::SendMail(run.dir.clone(), step));
        }
        if let Some(step) = out.retry {
            self.apply_flow_act(Act::Retry(run.dir.clone(), step));
        }
    }

    /// The graph and, at its right, the inspector.
    fn flow_body(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        (editor, run, error): (&mut FlowEditor, Option<&Run>, Option<&CheckError>),
    ) {
        let full = ui.available_rect_before_wrap();
        let inspect = full.width() >= SPLIT_WIDTH || editor.selected.is_some() || editor.rules_open;
        let width = inspector::WIDTH.min(full.width() * 0.6);
        let beside = if inspect { width } else { 0.0 };
        let split = full.right() - beside;
        let graph = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
        let mut graph_ui = ui.new_child(egui::UiBuilder::new().max_rect(graph));
        self.flow_graph((&mut graph_ui, beside), path, (editor, run, error));
        if inspect {
            let colors = Colors::of(&self.theme);
            let panel = egui::Rect::from_min_max(egui::pos2(split, full.top()), full.max);
            ui.painter()
                .rect_filled(panel, egui::CornerRadius::ZERO, colors.node);
            ui.painter()
                .vline(split, full.y_range(), egui::Stroke::new(1.0, colors.border));
            let mut panel_ui = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(12.0)));
            let show = inspector::Show {
                colors,
                run: run.map(|run| &run.state.steps),
                error,
            };
            if let Some(session) = inspector::show(&mut panel_ui, editor, &show) {
                self.go_session(&session);
            }
        }
        ui.allocate_rect(full, egui::Sense::hover());
    }

    /// The run strip of `run`, or "Starting" after a Run click.
    fn flow_run_strip(&mut self, ui: &mut egui::Ui, editor: &mut FlowEditor, run: Option<&Run>) {
        let colors = Colors::of(&self.theme);
        let Some(run) = run else {
            return;
        };
        if self.flows.banner_hidden.contains(&run.dir) {
            return;
        }
        match run_strip(ui, run, colors, &mut self.flows.drafts) {
            Some(StripClick::Hide) => {
                self.flows.banner_hidden.insert(run.dir.clone());
            }
            Some(StripClick::ExportReport) => {
                self.apply_flow_act(Act::ExportReport(run.dir.clone()));
            }
            Some(StripClick::Gate(step, choice)) => {
                self.apply_flow_act(Act::Gate(run.dir.clone(), step, choice));
            }
            Some(StripClick::Answer(step)) => {
                self.apply_flow_act(Act::Answer(run.dir.clone(), step));
            }
            Some(StripClick::Retry(step)) => {
                self.apply_flow_act(Act::Retry(run.dir.clone(), step));
            }
            Some(StripClick::Cancel) => {
                let at = ui.ctx().input(|input| input.pointer.interact_pos());
                editor.confirm_cancel = Some(at.unwrap_or_default());
            }
            None => {}
        }
    }

    /// The editor of a read flow. False when the user discarded it: the
    /// next frame reads the file again.
    fn flow_ready_view(&mut self, ui: &mut egui::Ui, path: &Path, editor: &mut FlowEditor) -> bool {
        editor.warned = false;
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
        self.flow_pane_keys(ui.ctx(), (path, editor));
        let mut run = self.flow_run_of(ui.ctx(), editor);
        // After a Run click, an older run of this flow is not the news.
        let starting = still_starting(&mut self.flows.starting, path, run.as_ref());
        if starting {
            run = None;
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        let colors = Colors::of(&self.theme);
        let keys = Keys {
            save: self.shortcut_label(Action::EditorSave),
            fit: self.shortcut_label(Action::FlowFit),
        };
        let running = run.as_ref().is_some_and(|run| !run.ended());
        let mut click = toolbar(ui, editor, (colors, &keys, running));
        if let Some(error) = editor.error.clone() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.colored_label(ui.visuals().error_fg_color, &error);
                if editor.disk_conflict {
                    if ui.small_button("Reload").clicked() {
                        click = Some(Click::Reload);
                    }
                    if ui.small_button("Overwrite anyway").clicked() {
                        click = Some(Click::OverwriteSave);
                    }
                }
            });
        }
        if starting {
            starting_strip(ui, colors);
        }
        self.flow_run_strip(ui, editor, run.as_ref());
        let error = check_error(&editor.draft());
        self.flow_body(ui, path, (editor, run.as_ref(), error.as_ref()));
        click = click.or(self.flow_editor_dialog(ui.ctx(), editor));
        self.flow_cancel_confirm(ui.ctx(), editor, run.as_ref());
        let keep = self.flow_editor_click((ui.ctx(), path), editor, click);
        if keep && !ui.ctx().input(|input| input.pointer.any_down()) {
            editor.sync_history();
        }
        keep
    }

    /// Does one click. False for Discard.
    fn flow_editor_click(
        &mut self,
        (ctx, path): (&egui::Context, &Path),
        editor: &mut FlowEditor,
        click: Option<Click>,
    ) -> bool {
        match click {
            Some(Click::Save) => editor.save(),
            Some(Click::OverwriteSave) => editor.save_with_force(true),
            Some(Click::Undo) => {
                editor.undo();
            }
            Some(Click::Redo) => {
                editor.redo();
            }
            Some(Click::Run) => self.flow_editor_run(path, editor),
            Some(Click::AddStep | Click::AddGate) => {
                let place = next_place(editor);
                let gate = click == Some(Click::AddGate);
                editor.selected = Some(viewer::add_node(&mut editor.snarl, place, gate));
                editor.fit = true;
            }
            Some(Click::Fit) => editor.fit = true,
            Some(Click::Tidy) => tidy(editor),
            Some(Click::Rules) => {
                editor.selected = None;
                editor.rules_open = true;
            }
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
            Some(Click::Discard | Click::Reload) => return false,
            None => {}
        }
        true
    }

    /// Run: an explicit click on the saved flow. The editor shows the run,
    /// so the app stays on it.
    fn flow_editor_run(&mut self, path: &Path, editor: &FlowEditor) {
        let Some(project) = self.flow_project(editor) else {
            return;
        };
        let loaded = LoadedFlows {
            project,
            root: editor.root.clone(),
            files: Vec::new(),
        };
        let inputs: Vec<_> = editor.saved.inputs.clone().into_iter().collect();
        let origin = crate::flows::inputs_form::Origin::Editor(path.into());
        self.request_flow_run(&loaded, &editor.file, (&inputs, origin));
    }
}

/// Tidy layout: the cards in columns by their order, with no overlap. The
/// new places are a layout change ("Layout not saved").
fn tidy(editor: &mut FlowEditor) {
    let sizes = editor.sizes.clone();
    model::tidy(&mut editor.snarl, |node| sizes.get(&node).copied());
    editor.fit = true;
}

/// A card whose height changed this frame (a run's note, Retry or a badge
/// came or went) moves the cards it now covers down, so no card hides
/// another. A drag changes no height, so it moves nothing.
fn grown_apart(editor: &mut FlowEditor, before: &canvas::Sizes, ctx: &egui::Context) {
    let grown = editor.sizes.iter().any(|(node, size)| {
        before
            .get(node)
            .is_none_or(|old| (old.y - size.y).abs() > 1.0)
    });
    let sizes = &editor.sizes;
    if grown && model::push_apart(&mut editor.snarl, |node| sizes.get(&node).copied()) {
        ctx.request_repaint();
    }
}

/// Where a node from the toolbar goes: below the lowest card.
fn next_place(editor: &FlowEditor) -> egui::Pos2 {
    let all = canvas::bounds(&editor.snarl, &editor.sizes);
    if all.is_finite() {
        egui::pos2(all.left(), all.bottom() + 40.0)
    } else {
        egui::Pos2::ZERO
    }
}
