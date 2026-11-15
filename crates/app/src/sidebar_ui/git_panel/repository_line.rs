//! The line above the Git (or Perforce) panel: a folder icon and the
//! repository name. When the terminal has not confirmed its directory, it
//! adds "Last known directory", so the panel says it may show an older
//! folder.
use super::*;

/// Text after the folder icon: the folder's own name, else its full path.
pub(crate) fn repository_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The repository line. `folder` is the repository root (or the
/// directory when there is none); `unconfirmed` is true when the session
/// has not confirmed that directory.
pub(crate) fn repository_line(ui: &mut egui::Ui, folder: Option<&Path>, unconfirmed: bool) {
    let caption = || RichText::new("Last known directory").small().weak();
    let Some(folder) = folder else {
        if unconfirmed {
            ui.label(caption());
        }
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(appearance::TOOLBAR_ICON, appearance::TOOLBAR_ICON),
            egui::Sense::hover(),
        );
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect,
            icon: "Folder",
            size: appearance::TOOLBAR_ICON,
            tint: appearance::icon_color(),
        });
        if unconfirmed {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(caption()).on_hover_text(
                    "The terminal has not confirmed this directory; it is the last one known",
                );
                name_label(ui, folder);
            });
        } else {
            name_label(ui, folder);
        }
    });
}

/// The repository name, truncated; the full path on hover.
fn name_label(ui: &mut egui::Ui, folder: &Path) {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.with_layout(layout, |ui| {
        ui.add(egui::Label::new(RichText::new(repository_name(folder)).small()).truncate())
            .on_hover_text(folder.display().to_string());
    });
}
