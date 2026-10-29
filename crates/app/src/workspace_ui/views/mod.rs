//! Non-terminal pane bodies: image, browser, and Git views.
use super::super::*;
use crate::app::OpenFileModeInput;

mod browser;
mod commit_log;
mod revisions;

impl App {
    pub(super) fn image_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_images.insert(path.into());
        let mut as_text = false;
        let mut reload = false;
        let mut fit = false;
        let mut actual = false;
        appearance::wrapping_path_row(ui, &services::compact_path(path), |ui| {
            reload = appearance::sidebar_action(ui, "RefreshCw", "Reload image").clicked();
            as_text = appearance::sidebar_action(ui, "FileText", "Open as text").clicked();
            if appearance::sidebar_action(ui, "ExternalLink", "Open externally").clicked() {
                let _ = self.jobs.send(Job::External(path.into()));
            }
            if let Some(texture) = self
                .images
                .get(path)
                .and_then(|preview| preview.texture.as_ref())
            {
                ui.weak(format!(
                    "{} × {} pixels",
                    texture.size()[0],
                    texture.size()[1]
                ));
                let fit_btn = ui.button("Fit");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "image-fit", fit_btn.rect);
                fit = fit_btn.clicked();
                actual = ui.button("100%").clicked();
            }
        });
        if as_text {
            self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line: None,
                split: None,
                external: false,
                text: true,
            });
        }
        if reload && let Some(preview) = self.images.get_mut(path) {
            if let Some(cancel) = preview.cancellation.take() {
                cancel.cancel();
            }
            preview.loading = false;
            preview.error = None;
        }
        if !self.images.contains_key(path) && self.images.len() >= 8 {
            ui.weak("Close another image preview to load this image.");
            return;
        }
        let preview = self.images.entry(path.into()).or_default();
        if reload || (!preview.loading && preview.texture.is_none() && preview.error.is_none()) {
            self.image_generation = self.image_generation.wrapping_add(1);
            preview.generation = self.image_generation;
            preview.cancellation = self
                .image_jobs
                .try_send((path.into(), preview.generation))
                .ok();
            preview.loading = preview.cancellation.is_some();
            if !preview.loading {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        if fit {
            preview.scene = egui::Rect::NOTHING;
        }
        if actual && let Some(size) = preview.texture.as_ref().map(egui::TextureHandle::size_vec2) {
            let center = size.to_pos2();
            preview.scene = egui::Rect::from_center_size(
                egui::pos2(center.x * 0.5, center.y * 0.5),
                ui.available_size(),
            );
        }
        preview.show(ui);
    }

    pub(super) fn draw_unsaved_close_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some((target, ids, error)) = self.unsaved_close_prompt(sid) else {
            return;
        };
        let enabled = !self.editor_close_busy(&ids);
        let bar = appearance::unsaved_close_bar(
            ui,
            appearance::UnsavedCloseBar {
                theme: &self.theme,
                message: &error,
                enabled,
            },
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record_response(ui.ctx(), "Save and close", &bar.save);
            diagnostics::record_response(ui.ctx(), "Discard changes", &bar.discard);
            diagnostics::record_response(ui.ctx(), "Cancel", &bar.cancel);
        }
        if let Some(choice) = bar.choice() {
            self.apply_unsaved_close_choice(choice, target, ids);
        }
    }
}
