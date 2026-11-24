//! The content of the flow editor's dialog: the plan preview. The fields
//! of a card and the rules are in the inspector.
use super::FlowEditor;
use super::toolbar::Click;
use crate::*;

/// The plan in the app's Markdown view, with the copy and export buttons.
/// The saved plan file opens in a Markdown tab, where the Mermaid block
/// shows as a diagram.
pub(super) fn preview_dialog(ui: &mut egui::Ui, editor: &mut FlowEditor) -> Option<Click> {
    if let Err(error) = editor.draft().check() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Not a correct flow yet: {error}"),
        );
    }
    let plan = editor.plan();
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui_commonmark::CommonMarkViewer::new()
                .explicit_image_uri_scheme(true)
                .show(ui, &mut editor.preview_cache, &plan);
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

impl App {
    /// The open dialog of the editor: a modal in the center of the window.
    /// It takes the input; a click outside it, Esc or Done closes it.
    pub(super) fn flow_editor_dialog(
        &mut self,
        ctx: &egui::Context,
        editor: &mut FlowEditor,
    ) -> Option<Click> {
        let super::Dialog::Preview = editor.dialog?;
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
                ui.heading("Flow plan");
                ui.weak("The flow as Markdown text.");
                ui.add_space(12.0);
                click = preview_dialog(ui, editor);
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

    /// The confirmation of Cancel run, at the pointer of the click.
    pub(super) fn flow_cancel_confirm(
        &mut self,
        ctx: &egui::Context,
        editor: &mut FlowEditor,
        run: Option<&crate::flows::Run>,
    ) {
        let Some(at) = editor.confirm_cancel else {
            return;
        };
        let Some(run) = run.filter(|run| !run.ended()) else {
            editor.confirm_cancel = None;
            return;
        };
        let dialog = appearance::Confirm::new(
            format!("Cancel the run of {}?", run.state.flow),
            "No new step starts. Running terminals continue.",
        )
        .cancel(appearance::ConfirmButton::new("Keep running"))
        .destructive(appearance::ConfirmButton::new("Cancel run"));
        let window = egui::Window::new("Cancel run")
            .id(egui::Id::unique(("flow-cancel", &run.dir)))
            .default_pos(at);
        let Some(answer) = appearance::confirm(ctx, window, dialog, |_| {}) else {
            return;
        };
        editor.confirm_cancel = None;
        if answer == appearance::ConfirmAnswer::Destructive {
            self.apply_flow_act(crate::flows::panel::Act::Cancel(run.dir.clone()));
        }
    }
}

const DIALOG_WIDTH: f32 = 620.0;
