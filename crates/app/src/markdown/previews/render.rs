use super::super::blocks::BlockHeights;
use super::super::html::{Document, Link, usize_from_f32};
use super::{Preview, ShowSyncedInput};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer, newline};

/// Blocks painted beyond the visible part, above and below.
const PAINT_MARGIN: f32 = 1200.0;
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
            counts: (!document.diagram_file).then_some((document.words, document.headings)),
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
        // A one-diagram document, or an inline diagram picked by a click,
        // paints alone with pan and zoom.
        if let Some((texture, single)) = zoom_target(document, self.zoomed, &self.mermaid_textures)
        {
            self.diagram_view(ui, &texture, single);
            return None;
        }
        self.zoomed = None;
        // A link to a `#heading` scrolls when the heading paints: paint
        // every block until it did.
        let heading_scroll = egui_commonmark::heading_scroll_pending(&mut self.cache);
        let output = self.scroll_area(ui, sid, scroll_fraction).show(ui, |ui| {
            PreviewBody {
                document,
                cache: &mut self.cache,
                textures: &self.mermaid_textures,
                heights: &mut self.block_heights,
                generation: self.applied_generation,
                paint_all: heading_scroll,
                zoom: &mut self.zoomed,
            }
            .show(ui);
        });
        if self.zoomed.is_some() {
            self.diagram_scene = egui::Rect::NOTHING;
            ui.ctx().request_repaint();
        }
        self.cache_scroll_height = output.content_size.y;
        self.scroll_fraction = scrolled_fraction(&output);
        document.links.iter().find_map(|(url, target)| {
            (self.cache.get_link_hook(url) == Some(true))
                .then(|| target.clone())
                .flatten()
        })
    }

    /// One diagram in a pan/zoom scene, with Fit and 100% like an image tab.
    /// `single` hides Back: the document has nothing else to go back to.
    fn diagram_view(&mut self, ui: &mut egui::Ui, texture: &egui::TextureHandle, single: bool) {
        let size = texture.size_vec2();
        let mut fit = false;
        let mut actual = false;
        let mut back = false;
        ui.horizontal(|ui| {
            if !single {
                back = crate::appearance::sidebar_action(ui, "ArrowLeft", "Back to document")
                    .clicked();
            }
            fit = crate::appearance::sidebar_action(ui, "Maximize", "Fit to pane").clicked();
            actual =
                crate::appearance::sidebar_action(ui, "ZoomIn", "Actual size (100%)").clicked();
            ui.weak(format!(
                "{} × {} pixels",
                texture.size()[0],
                texture.size()[1]
            ));
        });
        if back {
            self.zoomed = None;
            return;
        }
        if fit || !self.diagram_scene.is_finite() || !self.diagram_scene.is_positive() {
            self.diagram_scene = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        }
        if actual {
            self.diagram_scene = egui::Rect::from_center_size(
                egui::pos2(size.x * 0.5, size.y * 0.5),
                ui.available_size(),
            );
        }
        let available = ui.available_size_before_wrap();
        if !available.is_finite() || available.x <= 0.0 || available.y <= 0.0 {
            return;
        }
        let response =
            egui::Scene::new()
                .zoom_range(0.01..=16.0)
                .show(ui, &mut self.diagram_scene, |ui| {
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size));
                });
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "mermaid-diagram", response.response.rect);
        let _ = response;
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

/// How far down the preview is scrolled, from 0 to 1.
fn scrolled_fraction(output: &egui::scroll_area::ScrollAreaOutput<()>) -> f32 {
    let scroll_range = (output.content_size.y - output.inner_rect.height()).max(0.0);
    if scroll_range > 0.0 {
        (output.state.unclamped_offset().y / scroll_range).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The diagram to paint alone, and whether it is the whole document.
fn zoom_target(
    document: &Document,
    zoomed: Option<usize>,
    textures: &[egui::TextureHandle],
) -> Option<(egui::TextureHandle, bool)> {
    let single = document.single_diagram;
    let index = if single { Some(0) } else { zoomed }?;
    textures
        .get(index)
        .cloned()
        .map(|texture| (texture, single))
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
    /// `(words, headings)`; `None` for a Mermaid file.
    counts: Option<(usize, usize)>,
    status: &'a str,
}

impl StatusBar<'_> {
    fn show(self, ui: &mut egui::Ui) {
        let StatusBar {
            sid,
            counts,
            status,
        } = self;
        egui::Panel::bottom(egui::Id::unique(("markdown-status", sid)))
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| {
                let label = status_label(counts, status, ui.available_width());
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

/// The status text: counts first when there is room and `counts` is set.
pub(super) fn status_label(counts: Option<(usize, usize)>, status: &str, width: f32) -> String {
    match counts {
        Some((words, headings)) if width >= 360.0 => {
            format!("{words} words  ·  {headings} headings  ·  {status}")
        }
        _ => status.to_owned(),
    }
}

/// The rendered document: a centered column of Markdown text with Mermaid
/// diagrams painted in place of their source blocks.
struct PreviewBody<'a> {
    document: &'a Document,
    cache: &'a mut CommonMarkCache,
    textures: &'a [egui::TextureHandle],
    heights: &'a mut BlockHeights,
    /// Identifies the document for `heights`.
    generation: u64,
    /// Paint every block (a heading scroll is pending).
    paint_all: bool,
    /// Set to a diagram's index when a click asks to zoom it.
    zoom: &'a mut Option<usize>,
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
                    self.heights.begin(self.generation, ui.available_width());
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
        let mut position = 0;
        for (index, (diagram, texture)) in document.mermaid.iter().zip(self.textures).enumerate() {
            self.show_range(ui, position..diagram.range.start);
            let response = ui
                .add(
                    egui::Image::from_texture(texture)
                        .max_size(egui::vec2(ui.available_width(), 1200.0))
                        .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::ZoomIn)
                .on_hover_text("Click to zoom");
            if response.clicked() {
                *self.zoom = Some(index);
            }
            position = diagram.range.end;
        }
        self.show_range(ui, position..document.text.len());
    }

    /// The text in `range`, one top-level block at a time. A block far
    /// outside the view whose height is known only takes that height.
    fn show_range(&mut self, ui: &mut egui::Ui, range: std::ops::Range<usize>) {
        let document = self.document;
        let visible = ui.clip_rect().expand2(egui::vec2(0.0, PAINT_MARGIN));
        let mut first = true;
        for block in &document.blocks {
            let start = block.start.max(range.start);
            let end = block.end.min(range.end);
            let Some(text) = document
                .text
                .get(start..end)
                .filter(|text| !text.trim().is_empty())
            else {
                continue;
            };
            // Separate viewer calls drop the blank line the viewer puts
            // between blocks: one ends a block, one starts the next.
            if !first {
                newline(ui);
                newline(ui);
            }
            first = false;
            let top = ui.cursor().min.y;
            if let Some(height) = self.heights.get(start)
                && !self.paint_all
                && (top + height < visible.min.y || top > visible.max.y)
            {
                ui.add_space(height);
                continue;
            }
            self.show_text(ui, text);
            self.heights.set(start, ui.cursor().min.y - top);
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

#[cfg(test)]
#[test]
fn a_mermaid_file_status_has_no_word_or_heading_counts() {
    let snapshot = super::super::source::Snapshot {
        path: crate::test_path("/docs/flow.mmd"),
        text: "flowchart LR\n  A -->|Yes| B\n".into(),
        revision: None,
        paused: false,
    };
    let document = super::super::html::prepare(snapshot);
    assert!(document.diagram_file);
    let counts = (!document.diagram_file).then_some((document.words, document.headings));
    assert_eq!(status_label(counts, "Live preview", 800.0), "Live preview");
    assert_eq!(
        status_label(Some((21, 0)), "Live preview", 800.0),
        "21 words  ·  0 headings  ·  Live preview"
    );
}
