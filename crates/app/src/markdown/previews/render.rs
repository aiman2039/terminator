use super::super::html::{Document, Link, usize_from_f32};
use super::{Preview, ShowSyncedInput};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::sync::Arc;

impl Preview {
    pub fn show_synced(&mut self, input: ShowSyncedInput<'_>) -> Option<Link> {
        let ShowSyncedInput {
            ui,
            sid,
            status_override,
            scroll_fraction,
        } = input;
        self.preview_rect = ui.max_rect();
        self.pointer_focus(ui);
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "markdown-preview", self.preview_rect);
        if let Some(error) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Preview could not refresh: {error}"),
            );
        }
        let Some(document) = &self.document else {
            if self.error.is_none() {
                ui.spinner();
            }
            return None;
        };
        let status = self.status_text(document, status_override);
        if self.mermaid_textures.len() != document.mermaid.len() {
            self.mermaid_textures = mermaid_textures(ui.ctx(), document, sid);
        }
        StatusBar {
            sid,
            words: document.words,
            headings: document.headings,
            status,
        }
        .show(ui);
        #[cfg(feature = "test-support")]
        {
            crate::diagnostics::record(
                ui.ctx(),
                &format!("markdown-status:{status}"),
                self.preview_rect,
            );
        }
        let output = self.scroll_area(ui, sid, scroll_fraction).show(ui, |ui| {
            PreviewBody {
                document,
                cache: &mut self.cache,
                textures: &self.mermaid_textures,
            }
            .show(ui);
        });
        self.cache_scroll_height = output.content_size.y;
        let scroll_range = (output.content_size.y - output.inner_rect.height()).max(0.0);
        self.scroll_fraction = if scroll_range > 0.0 {
            (output.state.unclamped_offset().y / scroll_range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        document.links.iter().find_map(|(url, target)| {
            (self.cache.get_link_hook(url) == Some(true))
                .then(|| target.clone())
                .flatten()
        })
    }

    fn status_text<'a>(&self, document: &'a Document, status_override: Option<&'a str>) -> &'a str {
        if self.error.is_some() {
            "Last available preview"
        } else if self.snapshot_source.is_some()
            && self.applied_generation != self.snapshot_generation
        {
            "Updating preview…"
        } else {
            status_override.unwrap_or(document.status)
        }
    }

    fn scroll_area(
        &self,
        ui: &egui::Ui,
        sid: &str,
        scroll_fraction: Option<f32>,
    ) -> egui::ScrollArea {
        let scroll = egui::ScrollArea::both()
            .id_salt(("markdown-scroll", sid))
            .auto_shrink([false, false]);
        let Some(fraction) = scroll_fraction else {
            return scroll;
        };
        let content_height = self.cache_scroll_height;
        let viewport_height = ui.available_height();
        scroll.vertical_scroll_offset(
            (content_height - viewport_height).max(0.0) * fraction.clamp(0.0, 1.0),
        )
    }
}

fn mermaid_textures(
    ctx: &egui::Context,
    document: &Document,
    sid: &str,
) -> Vec<egui::TextureHandle> {
    document
        .mermaid
        .iter()
        .enumerate()
        .map(|(index, diagram)| {
            ctx.load_texture(
                format!("markdown-mermaid:{sid}:{index}"),
                Arc::clone(&diagram.image),
                egui::TextureOptions::LINEAR,
            )
        })
        .collect()
}

/// Word and heading counts plus the preview status, bottom right.
struct StatusBar<'a> {
    sid: &'a str,
    words: usize,
    headings: usize,
    status: &'a str,
}

impl StatusBar<'_> {
    fn show(self, ui: &mut egui::Ui) {
        let StatusBar {
            sid,
            words,
            headings,
            status,
        } = self;
        egui::Panel::bottom(egui::Id::unique(("markdown-status", sid)))
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| {
                let label = if ui.available_width() < 360.0 {
                    status.to_owned()
                } else {
                    format!("{words} words  ·  {headings} headings  ·  {status}")
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::Frame::new()
                        .fill(ui.visuals().panel_fill)
                        .corner_radius(crate::appearance::MENU_RADIUS)
                        .inner_margin(egui::Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.weak(label);
                        });
                });
            });
    }
}

/// The rendered document: a centered column of Markdown text with Mermaid
/// diagrams painted in place of their source blocks.
struct PreviewBody<'a> {
    document: &'a Document,
    cache: &'a mut CommonMarkCache,
    textures: &'a [egui::TextureHandle],
}

impl PreviewBody<'_> {
    fn show(mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        let width = ui.available_width().min(720.0);
        let margin = ((ui.available_width() - width) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_max_width(width);
                egui::Frame::NONE.inner_margin(18).show(ui, |ui| {
                    Self::apply_style(ui);
                    self.content(ui);
                });
            });
        });
    }

    fn apply_style(ui: &mut egui::Ui) {
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
        ui.style_mut().text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(28.0, egui::FontFamily::Name("Semibold".into())),
        );
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
        ui.spacing_mut().item_spacing.y = 10.0;
        // Body text between the theme's text and strong colors: easier to
        // read than the default gray, still below headings.
        let visuals = ui.visuals_mut();
        let strong = visuals.strong_text_color();
        let text = &mut visuals.widgets.noninteractive.fg_stroke.color;
        *text = text.lerp_to_gamma(strong, 0.6);
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let document = self.document;
        if document.mermaid.is_empty() {
            self.show_text(ui, &document.text);
            return;
        }
        let mut position = 0;
        for (diagram, texture) in document.mermaid.iter().zip(self.textures) {
            if let Some(before) = document.text.get(position..diagram.range.start)
                && !before.is_empty()
            {
                self.show_text(ui, before);
            }
            ui.add(
                egui::Image::from_texture(texture)
                    .max_size(egui::vec2(ui.available_width(), 1200.0)),
            );
            position = diagram.range.end;
        }
        if let Some(after) = document.text.get(position..)
            && !after.is_empty()
        {
            self.show_text(ui, after);
        }
    }

    fn show_text(&mut self, ui: &mut egui::Ui, text: &str) {
        CommonMarkViewer::new()
            .explicit_image_uri_scheme(true)
            .max_image_width(Some(usize_from_f32(ui.available_width().max(1.0))))
            .enable_scroll_to_heading(true)
            .show(ui, self.cache, text);
    }
}
