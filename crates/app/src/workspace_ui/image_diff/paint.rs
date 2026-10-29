//! Painting an image comparison: captions, mode toggles, and the canvas.
use super::{ImageDiff, Mode, Shown};
use crate::appearance;
use eframe::egui::{self, Color32, TextureHandle};

/// Smallest canvas height, so a short pane still shows the images.
const MIN_CANVAS: f32 = 160.0;

impl ImageDiff {
    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        let sides = match &self.sides {
            None => {
                ui.spinner();
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
            Some(Ok(sides)) => sides,
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (mode, icon, tip) in [
                (Mode::SideBySide, "Columns2", "Side by side"),
                (Mode::Swipe, "Split", "Swipe"),
                (Mode::Onion, "Eye", "Onion skin"),
            ] {
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon,
                    tip,
                    selected: self.mode == mode,
                })
                .clicked()
                {
                    self.mode = mode;
                }
            }
            if self.mode != Mode::SideBySide {
                ui.add(egui::Slider::new(&mut self.mix, 0.0..=1.0).show_value(false));
            }
        });
        ui.horizontal(|ui| {
            ui.weak(caption("Old", &sides[0]));
            ui.separator();
            ui.weak(caption("New", &sides[1]));
        });
        let size = egui::vec2(ui.available_width(), ui.available_height().max(MIN_CANVAS));
        let (canvas, response) = ui.allocate_exact_size(size, egui::Sense::drag());
        let [old, new] = sides;
        let (old, new) = (texture(old), texture(new));
        let mix = self.mix;
        match self.mode {
            Mode::SideBySide => {
                let half = canvas.width() / 2.0;
                let left = canvas.with_max_x(canvas.left() + half - 4.0);
                let right = canvas.with_min_x(canvas.left() + half + 4.0);
                side(ui, left, old);
                side(ui, right, new);
            }
            Mode::Swipe => {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.mix = ((pointer.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0);
                }
                swipe(ui, canvas, [old, new], mix);
            }
            Mode::Onion => onion(ui, canvas, [old, new], mix),
        }
    }
}

fn caption(name: &str, side: &Shown) -> String {
    let Some(texture) = &side.texture else {
        return format!("{name}: none");
    };
    let dims = side
        .dims
        .map(|[width, height]| format!(" · {width}×{height}"))
        .unwrap_or_default();
    let error = match texture {
        Err(error) => format!(" · {error}"),
        Ok(_) => String::new(),
    };
    format!(
        "{name}: {}{dims}{error}",
        crate::session_info::format_bytes(side.bytes as u64)
    )
}

fn texture(side: &Shown) -> Option<&TextureHandle> {
    side.texture
        .as_ref()
        .and_then(|texture| texture.as_ref().ok())
}

/// `size` scaled to fit `rect`, centered.
fn fit(rect: egui::Rect, size: egui::Vec2) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    egui::Rect::from_center_size(rect.center(), egui::vec2(size.x * scale, size.y * scale))
}

fn paint(
    ui: &egui::Ui,
    (clip, rect): (egui::Rect, egui::Rect),
    texture: &TextureHandle,
    alpha: f32,
) {
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    ui.painter().with_clip_rect(clip).image(
        texture.id(),
        rect,
        uv,
        Color32::WHITE.gamma_multiply(alpha),
    );
}

fn side(ui: &egui::Ui, rect: egui::Rect, texture: Option<&TextureHandle>) {
    ui.painter().rect_stroke(
        rect,
        2.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    if let Some(texture) = texture {
        paint(ui, (rect, fit(rect, texture.size_vec2())), texture, 1.0);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No image",
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
    }
}

/// One frame both sides share: the larger of the two sizes.
fn shared_frame(canvas: egui::Rect, sides: [Option<&TextureHandle>; 2]) -> egui::Rect {
    let size = sides
        .iter()
        .flatten()
        .map(|texture| texture.size_vec2())
        .fold(egui::Vec2::ZERO, egui::Vec2::max);
    fit(canvas, size)
}

/// Old on the left of the split, new on the right.
fn swipe(ui: &egui::Ui, canvas: egui::Rect, sides: [Option<&TextureHandle>; 2], mix: f32) {
    let frame = shared_frame(canvas, sides);
    let split = frame.left() + frame.width() * mix;
    let [old, new] = sides;
    if let Some(old) = old {
        paint(ui, (frame.with_max_x(split), frame), old, 1.0);
    }
    if let Some(new) = new {
        paint(ui, (frame.with_min_x(split), frame), new, 1.0);
    }
    ui.painter().vline(
        split,
        frame.y_range(),
        egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
    );
}

/// New over old at `mix` opacity.
fn onion(ui: &egui::Ui, canvas: egui::Rect, sides: [Option<&TextureHandle>; 2], mix: f32) {
    let frame = shared_frame(canvas, sides);
    let [old, new] = sides;
    if let Some(old) = old {
        paint(ui, (frame, frame), old, 1.0);
    }
    if let Some(new) = new {
        paint(ui, (frame, frame), new, mix);
    }
}
