//! Non-terminal pane bodies: image, browser, and Git views.
use super::super::*;
use crate::app::OpenFileModeInput;

mod commit_log;
mod revisions;

/// What the browser toolbar's controls asked for this frame.
#[derive(Default)]
struct BrowserBar {
    as_text: bool,
    system: bool,
    reload: bool,
    back: bool,
    forward: bool,
    go: bool,
}

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

    pub(super) fn browser_view(&mut self, ui: &mut egui::Ui, key: String, target: &BrowserTarget) {
        let href = crate::browser::href(target).unwrap_or_else(|_| target.title());
        let mut draft = self
            .browser_urls
            .remove(&key)
            .unwrap_or_else(|| href.clone());
        if draft.is_empty() {
            draft = href;
        }
        let bar = Self::browser_toolbar(ui, &mut draft, target);
        self.apply_browser_bar(&key, target, (draft, bar));
        if let Some(error) = self.browser_host.error(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.weak("Open in browser to view the page in your system browser.");
        }
        let rect = ui.available_rect_before_wrap();
        let _ = ui.allocate_rect(rect, egui::Sense::hover());
        self.visible_browsers
            .push(crate::browser_host::VisibleBrowser {
                key,
                target: target.clone(),
                rect,
            });
    }

    fn browser_toolbar(
        ui: &mut egui::Ui,
        draft: &mut String,
        target: &BrowserTarget,
    ) -> BrowserBar {
        let mut bar = BrowserBar::default();
        ui.horizontal_wrapped(|ui| {
            bar.back = appearance::sidebar_action(ui, "ArrowLeft", "Back").clicked();
            bar.forward = appearance::sidebar_action(ui, "ArrowRight", "Forward").clicked();
            bar.reload = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
            let url = ui.add(
                appearance::singleline(draft)
                    .desired_width(240.0)
                    .hint_text("https://"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-url", url.rect);
            let go_btn = ui.button("Go");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-go", go_btn.rect);
            bar.go = go_btn.clicked()
                || (url.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            bar.as_text = target.file().is_some()
                && appearance::sidebar_action(ui, "FileText", "Open as text").clicked();
            let open = appearance::sidebar_action(ui, "ExternalLink", "Open in system browser");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "html-open-browser", open.rect);
            bar.system = open.clicked();
        });
        bar
    }

    fn apply_browser_bar(
        &mut self,
        key: &str,
        target: &BrowserTarget,
        (draft, bar): (String, BrowserBar),
    ) {
        if bar.go
            && let Ok(next) = BrowserTarget::from_http_url(&draft)
            && next != *target
        {
            self.browser_submit = Some((key.to_owned(), next));
        }
        self.browser_urls.insert(key.to_owned(), draft);
        if bar.back {
            self.browser_host.go_back(key);
        }
        if bar.forward {
            self.browser_host.go_forward(key);
        }
        if bar.reload && !self.browser_host.reload_view(key) {
            self.browser_host.drop_view(key);
        }
        if bar.as_text
            && let Some(path) = target.file()
        {
            self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line: None,
                split: None,
                external: false,
                text: true,
            });
        }
        if bar.system {
            match target {
                BrowserTarget::File(path) => self.open_in_browser(path),
                BrowserTarget::Url(url) => {
                    let _ = self.jobs.send(Job::Browser(url.clone()));
                }
            }
        }
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
