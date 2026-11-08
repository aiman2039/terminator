//! The remote app's pointer shape over its tab: a named shape becomes the
//! local system cursor; an image is drawn at the pointer with the system
//! cursor hidden.

use base64::Engine as _;
use eframe::egui::{self, ColorImage, CursorIcon, Pos2, Rect, TextureOptions, Vec2};
use terminator_core::remote_apps::RemoteCursor;

/// One project's cursor, rebuilt only when its serial changes.
#[derive(Default)]
pub(super) struct CursorView {
    serial: u64,
    shape: Shape,
}

#[derive(Default)]
enum Shape {
    /// No cursor from the app yet: the normal arrow.
    #[default]
    System,
    Named(CursorIcon),
    Hidden,
    Image {
        texture: egui::TextureHandle,
        hotspot: Vec2,
    },
}

impl CursorView {
    pub(super) fn update(&mut self, ctx: &egui::Context, cursor: Option<&RemoteCursor>) {
        let Some(cursor) = cursor else {
            return;
        };
        if cursor.serial == self.serial {
            return;
        }
        self.serial = cursor.serial;
        self.shape = shape(ctx, cursor);
    }

    /// Applies the cursor while the pointer is at `pos` over a remote window.
    /// `scale` is app pixels per point.
    #[expect(clippy::arithmetic_side_effects, reason = "f32 screen geometry.")]
    pub(super) fn show(&self, ui: &egui::Ui, pos: Pos2, scale: f32) {
        let ctx = ui.ctx();
        match &self.shape {
            Shape::System => {}
            Shape::Named(icon) => ctx.set_cursor_icon(*icon),
            Shape::Hidden => ctx.set_cursor_icon(CursorIcon::None),
            Shape::Image { texture, hotspot } => {
                ctx.set_cursor_icon(CursorIcon::None);
                let size = texture.size_vec2() / scale;
                let rect = Rect::from_min_size(pos - *hotspot / scale, size);
                let layer =
                    egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("remote-cursor"));
                ctx.layer_painter(layer).image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
    }
}

fn shape(ctx: &egui::Context, cursor: &RemoteCursor) -> Shape {
    if let Some(name) = &cursor.name {
        return Shape::Named(named(name));
    }
    let size = [usize::from(cursor.width), usize::from(cursor.height)];
    let Ok(rgba) = base64::engine::general_purpose::STANDARD.decode(&cursor.rgba) else {
        return Shape::System;
    };
    if size.contains(&0) || rgba.len() != size[0].saturating_mul(size[1]).saturating_mul(4) {
        return Shape::Hidden;
    }
    let image = ColorImage::from_rgba_premultiplied(size, &rgba);
    #[expect(
        clippy::cast_precision_loss,
        reason = "Cursor hotspots are small pixel offsets."
    )]
    let hotspot = Vec2::new(cursor.hotspot_x as f32, cursor.hotspot_y as f32);
    Shape::Image {
        texture: ctx.load_texture("remote-cursor", image, TextureOptions::NEAREST),
        hotspot,
    }
}

/// CSS cursor names (what Wayland apps use) to egui cursors.
pub(super) fn named(name: &str) -> CursorIcon {
    match name {
        "none" => CursorIcon::None,
        "context-menu" => CursorIcon::ContextMenu,
        "help" => CursorIcon::Help,
        "pointer" | "hand1" | "hand2" => CursorIcon::PointingHand,
        "progress" => CursorIcon::Progress,
        "wait" | "watch" => CursorIcon::Wait,
        "cell" => CursorIcon::Cell,
        "crosshair" => CursorIcon::Crosshair,
        "text" | "xterm" => CursorIcon::Text,
        "vertical-text" => CursorIcon::VerticalText,
        "alias" => CursorIcon::Alias,
        "copy" => CursorIcon::Copy,
        "move" => CursorIcon::Move,
        "all-scroll" => CursorIcon::AllScroll,
        "no-drop" => CursorIcon::NoDrop,
        "not-allowed" => CursorIcon::NotAllowed,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "col-resize" | "ew-resize" => CursorIcon::ResizeColumn,
        "row-resize" | "ns-resize" => CursorIcon::ResizeRow,
        "e-resize" => CursorIcon::ResizeEast,
        "w-resize" => CursorIcon::ResizeWest,
        "n-resize" => CursorIcon::ResizeNorth,
        "s-resize" => CursorIcon::ResizeSouth,
        "ne-resize" => CursorIcon::ResizeNorthEast,
        "nw-resize" => CursorIcon::ResizeNorthWest,
        "se-resize" => CursorIcon::ResizeSouthEast,
        "sw-resize" => CursorIcon::ResizeSouthWest,
        "nesw-resize" => CursorIcon::ResizeNeSw,
        "nwse-resize" => CursorIcon::ResizeNwSe,
        "zoom-in" => CursorIcon::ZoomIn,
        "zoom-out" => CursorIcon::ZoomOut,
        _ => CursorIcon::Default,
    }
}
