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
