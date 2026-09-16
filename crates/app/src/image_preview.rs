//! Native, read-only media tabs. Decode on a worker, never in a paint callback.
use anyhow::{Context, Result, ensure};
use eframe::egui::{self, ColorImage, TextureHandle};
use std::{
    io::{Cursor, Read},
    path::Path,
};

const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_SIDE: u32 = 16384;

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "tif" | "tiff" | "svg"
            )
        })
}

pub fn decode(path: &Path) -> Result<ColorImage> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .context("Open image")?;
    ensure!(file.metadata()?.is_file(), "Image is not a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "Image exceeds the 32 MiB file limit"
    );
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
    {
        // resvg supplies a self-contained rasterizer. Resolve no external files.
        // Constrain the output independently of untrusted SVG dimensions.
        let mut options = resvg::usvg::Options::default();
        let omitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let linked = omitted.clone();
        options.image_href_resolver.resolve_string = Box::new(move |_, _| {
            linked.store(true, std::sync::atomic::Ordering::Relaxed);
            None
        });
        // Validate embedded raster payloads with the same decoder limits, and
        // share a total pixel budget across all images in the SVG.
        let remaining = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(MAX_PIXELS));
        let embedded = omitted.clone();
        let resolver = resvg::usvg::ImageHrefResolver::default().resolve_data;
        options.image_href_resolver.resolve_data = Box::new(move |mime, data, options| {
            let valid = raster(&data).ok().is_some_and(|image| {
                let pixels = u64::from(image.width()) * u64::from(image.height());
                remaining
                    .fetch_update(
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                        |n| n.checked_sub(pixels),
                    )
                    .is_ok()
            });
            if valid {
                resolver(mime, data, options)
            } else {
                embedded.store(true, std::sync::atomic::Ordering::Relaxed);
                None
            }
        });
        options.fontdb_mut().load_system_fonts();
        let decoded = egui_extras::image::load_svg_bytes_with_size(
            &bytes,
            egui::SizeHint::Size {
                width: 2048,
                height: 2048,
                maintain_aspect_ratio: true,
            },
            &options,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            !omitted.load(std::sync::atomic::Ordering::Relaxed),
            "SVG contains unsupported or external image references; open externally to view the complete document"
        );
        return Ok(decoded);
    }
    let rgba = raster(&bytes)?.into_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}
fn raster(bytes: &[u8]) -> Result<image::DynamicImage> {
    use image::{ImageDecoder, ImageReader};
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "Image exceeds the 16 megapixel preview limit"
    );
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub struct Preview {
    pub texture: Option<TextureHandle>,
    pub error: Option<String>,
    pub loading: bool,
    pub generation: u64,
    pub scene: egui::Rect,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            texture: None,
            error: None,
            loading: false,
            generation: 0,
            scene: egui::Rect::NOTHING,
        }
    }
}
impl Preview {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(texture) = &self.texture {
            let size = texture.size_vec2();
            let response =
                egui::Scene::new()
                    .zoom_range(0.01..=16.0)
                    .show(ui, &mut self.scene, |ui| {
                        ui.add(egui::Image::new(texture).fit_to_exact_size(size));
                    });
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), "image-preview", response.response.rect);
            let _ = response;
        } else if let Some(error) = &self.error {
            let response = ui.colored_label(ui.visuals().error_fg_color, error);
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), "image-error", response.rect);
            let _ = response;
        } else {
            ui.spinner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    #[test]
    fn decodes_pixels_and_reports_corrupt_or_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("space image.PNG");
        let pixels = image::RgbaImage::from_pixel(8, 4, image::Rgba([20, 40, 60, 128]));
        pixels.save(&path).unwrap();
        let decoded = decode(&path).unwrap();
        assert_eq!(decoded.size, [8, 4]);
        assert_eq!(decoded.pixels[0].a(), 128);
        std::fs::write(&path, "not an image").unwrap();
        assert!(decode(&path).is_err());
        File::create(&path).unwrap().set_len(MAX_FILE + 1).unwrap();
        assert!(decode(&path).unwrap_err().to_string().contains("32 MiB"));
    }
    #[test]
    fn svg_renders_bounded_embedded_png_and_rejects_external_references() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("embedded.svg");
        let raster = image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 40, 60, 255]));
        let mut png = Cursor::new(Vec::new());
        raster.write_to(&mut png, image::ImageFormat::Png).unwrap();
        std::fs::write(&path,format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><image width="32" height="32" href="data:image/png;base64,{}"/></svg>"#,STANDARD.encode(png.into_inner()))).unwrap();
        let decoded = decode(&path).unwrap();
        let center = decoded.pixels[decoded.size[0] * decoded.size[1] / 2 + decoded.size[0] / 2];
        assert!(
            center.r() > 150 && center.a() == 255,
            "Embedded image was omitted"
        );
        std::fs::write(&path,r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><image width="32" height="32" href="/untrusted/file.png"/></svg>"#).unwrap();
        assert!(decode(&path).is_err());
    }
    #[test]
    fn svg_output_is_bounded_and_image_extensions_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.svg");
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="50000"><rect width="100000" height="50000" fill="red"/></svg>"#).unwrap();
        assert_eq!(decode(&path).unwrap().size, [2048, 1024]);
        assert!(supported(Path::new("file.JpEg")));
        assert!(!supported(Path::new("file.rs")));
    }
}
