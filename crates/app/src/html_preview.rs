//! GUI-only HTML preview. Raster on a worker, never in a paint callback.
use anyhow::{Context, Result, ensure};
use eframe::egui::ColorImage;
use std::{io::Read, path::Path, sync::Arc};

const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;
const MAX_HEIGHT: f64 = 4000.0;
const MIN_HEIGHT: f64 = 400.0;
const DEFAULT_WIDTH: u32 = 900;
const SCALE: f64 = 1.0;

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            )
        })
}

pub fn decode(path: &Path) -> Result<ColorImage> {
    raster_file(path, DEFAULT_WIDTH)
}

fn raster_file(path: &Path, css_width: u32) -> Result<ColorImage> {
    let html = read_html(path)?;
    let base = url::Url::from_file_path(path)
        .ok()
        .map(|url| url.to_string());
    raster_html(html, base, css_width)
}

fn read_html(path: &Path) -> Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .context("Open HTML")?;
    ensure!(file.metadata()?.is_file(), "HTML is not a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "HTML exceeds the 2 MiB preview limit"
    );
    String::from_utf8(bytes).context("HTML is not UTF-8")
}

fn raster_html(html: String, base_url: Option<String>, css_width: u32) -> Result<ColorImage> {
    use anyrender::{PaintScene as _, render_to_buffer};
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use blitz_dom::{DocumentConfig, StyleThreading, util::Color};
    use blitz_html::HtmlDocument;
    use blitz_paint::paint_scene;
    use blitz_traits::shell::{ColorScheme, Viewport};
    use peniko::Fill;
    use peniko::kurbo::Rect;

    let css_width = css_width.clamp(400, 1600);
    let scale = SCALE as f32;
    let viewport_w = (f64::from(css_width) * SCALE) as u32;
    let viewport_h = (MIN_HEIGHT * SCALE) as u32;
    let net: Arc<dyn blitz_traits::net::NetProvider> =
        Arc::new(blitz_traits::net::DummyNetProvider);
    let mut document = HtmlDocument::from_html(
        &html,
        DocumentConfig {
            base_url,
            net_provider: Some(net),
            viewport: Some(Viewport::new(
                viewport_w,
                viewport_h,
                scale,
                ColorScheme::Light,
            )),
            style_threading: StyleThreading::Sequential,
            ..DocumentConfig::default()
        },
    );
    document.resolve(0.0);
    let computed = f64::from(document.root_element().final_layout().size.height);
    let css_height = computed.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let render_width = (f64::from(css_width) * SCALE) as u32;
    let render_height = (css_height * SCALE) as u32;
    ensure!(
        render_width > 0 && render_height > 0,
        "HTML preview is empty"
    );
    ensure!(
        render_width <= MAX_SIDE && render_height <= MAX_SIDE,
        "HTML preview exceeds the {MAX_SIDE}px side limit"
    );
    ensure!(
        u64::from(render_width) * u64::from(render_height) <= MAX_PIXELS,
        "HTML preview exceeds the 16 megapixel limit"
    );
    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Default::default(),
                Color::WHITE,
                Default::default(),
                &Rect::new(0.0, 0.0, f64::from(render_width), f64::from(render_height)),
            );
            paint_scene(
                scene,
                document.as_mut(),
                SCALE,
                render_width,
                render_height,
                0,
                0,
            );
        },
        render_width,
        render_height,
    );
    Ok(ColorImage::from_rgba_premultiplied(
        [render_width as usize, render_height as usize],
        &buffer,
    ))
}

pub struct Preview {
    pub texture: Option<eframe::egui::TextureHandle>,
    pub error: Option<String>,
    pub loading: bool,
    pub generation: u64,
    pub scene: eframe::egui::Rect,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            texture: None,
            error: None,
            loading: false,
            generation: 0,
            scene: eframe::egui::Rect::NOTHING,
        }
    }
}

impl Preview {
    pub fn show(&mut self, ui: &mut eframe::egui::Ui) {
        if let Some(texture) = &self.texture {
            let size = texture.size_vec2();
            let mut actual_size = false;
            ui.horizontal(|ui| {
                ui.weak(format!(
                    "{} × {} pixels",
                    texture.size()[0],
                    texture.size()[1]
                ));
                if ui.button("Fit").clicked() {
                    self.scene = eframe::egui::Rect::NOTHING;
                }
                if ui.button("100%").clicked() {
                    actual_size = true;
                }
            });
            if actual_size {
                self.scene =
                    eframe::egui::Rect::from_center_size(size.to_pos2() * 0.5, ui.available_size());
            }
            let _ = eframe::egui::Scene::new().zoom_range(0.01..=16.0).show(
                ui,
                &mut self.scene,
                |ui| {
                    ui.add(eframe::egui::Image::new(texture).fit_to_exact_size(size));
                },
            );
        } else if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else {
            ui.spinner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_extensions_are_previewed() {
        assert!(supported(Path::new("docs/index.HTML")));
        assert!(supported(Path::new("page.xhtml")));
        assert!(!supported(Path::new("page.md")));
    }

    #[test]
    fn rasters_inline_html_and_rejects_oversize_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.html");
        std::fs::write(
            &path,
            "<!doctype html><title>Hi</title><body style=\"background:#2244aa;color:#fff\"><h1>Hello</h1></body>",
        )
        .unwrap();
        let image = decode(&path).unwrap();
        assert!(image.size[0] >= 400);
        assert!(image.size[1] >= 400);
        std::fs::write(&path, "x".repeat((MAX_FILE as usize) + 1)).unwrap();
        assert!(decode(&path).unwrap_err().to_string().contains("2 MiB"));
    }
}
