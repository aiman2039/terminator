//! Menu-bar icon for the macOS status item. The badge is Telegram red
//! (`#FF3B30`) with a white count. `99+` is the cap.

use std::sync::OnceLock;

const SCALE: u32 = 2;
const ICON_PX: u32 = 16 * SCALE;
const CANVAS: u32 = 18 * SCALE;
const BADGE: image::Rgba<u8> = image::Rgba([255, 59, 48, 255]);
const INK: image::Rgba<u8> = image::Rgba([255, 255, 255, 255]);

pub(crate) struct StatusIcon {
    pub png: Vec<u8>,
    pub width_pt: f32,
    pub height_pt: f32,
}

pub(crate) fn status_icon(waiting: usize) -> StatusIcon {
    let canvas = compose(waiting);
    let (width, height) = (canvas.width(), canvas.height());
    let mut png = Vec::new();
    canvas
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("status icon encodes");
    StatusIcon {
        png,
        width_pt: width as f32 / SCALE as f32,
        height_pt: height as f32 / SCALE as f32,
    }
}

fn base_icon() -> &'static image::RgbaImage {
    static ICON: OnceLock<image::RgbaImage> = OnceLock::new();
    ICON.get_or_init(|| {
        let decoded = image::load_from_memory(include_bytes!("../assets/branding/terminator.png"))
            .expect("bundled Terminator icon")
            .to_rgba8();
        image::imageops::resize(
            &decoded,
            ICON_PX,
            ICON_PX,
            image::imageops::FilterType::Lanczos3,
        )
    })
}

fn compose(waiting: usize) -> image::RgbaImage {
    let icon = base_icon();
    if waiting == 0 {
        let mut canvas = image::RgbaImage::new(CANVAS, CANVAS);
        image::imageops::overlay(&mut canvas, icon, 2, 2);
        return canvas;
    }
    let label = badge_label(waiting);
    let badge_h = 8 * SCALE;
    let badge_w = badge_width(&label, badge_h);
    let width = 2 + ICON_PX + badge_w - 6;
    let mut canvas = image::RgbaImage::new(width, CANVAS);
    image::imageops::overlay(&mut canvas, icon, 2, 2);
    let badge_x = width - badge_w;
    fill_pill(&mut canvas, badge_x, 0, badge_w, badge_h, BADGE);
    blit_label(&mut canvas, &label, badge_x, 0, badge_w, badge_h);
    canvas
}

fn badge_label(waiting: usize) -> String {
    if waiting > 99 {
        "99+".into()
    } else {
        waiting.to_string()
    }
}

fn badge_width(label: &str, badge_h: u32) -> u32 {
    if label.chars().count() == 1 {
        return badge_h;
    }
    let glyphs = label.chars().count() as u32;
    let text = glyphs * 5 * SCALE + glyphs.saturating_sub(1) * SCALE;
    text + 4 * SCALE
}

fn fill_pill(image: &mut image::RgbaImage, x: u32, y: u32, w: u32, h: u32, color: image::Rgba<u8>) {
    let radius = h as f32 / 2.0;
    let left = x as f32 + radius;
    let right = x as f32 + w as f32 - radius;
    let cy = y as f32 + radius;
    for py in y..y.saturating_add(h).min(image.height()) {
        for px in x..x.saturating_add(w).min(image.width()) {
            let dx = if (px as f32) + 0.5 < left {
                left - (px as f32 + 0.5)
            } else if (px as f32) + 0.5 > right {
                px as f32 + 0.5 - right
            } else {
                0.0
            };
            let dy = py as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= radius * radius {
                image.put_pixel(px, py, color);
            }
        }
    }
}

fn blit_label(image: &mut image::RgbaImage, label: &str, x: u32, y: u32, w: u32, h: u32) {
    let glyphs = label.chars().count() as u32;
    let text_w = glyphs * 5 * SCALE + glyphs.saturating_sub(1) * SCALE;
    let text_h = 7 * SCALE;
    let mut cursor = x + w.saturating_sub(text_w) / 2;
    let origin_y = y + h.saturating_sub(text_h) / 2;
    for ch in label.chars() {
        blit_glyph(image, glyph(ch), cursor, origin_y);
        cursor += 5 * SCALE + SCALE;
    }
}

fn blit_glyph(image: &mut image::RgbaImage, rows: &[&str], origin_x: u32, origin_y: u32) {
    for (row, line) in rows.iter().enumerate() {
        for (col, bit) in line.chars().enumerate() {
            if bit != '1' {
                continue;
            }
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let x = origin_x + col as u32 * SCALE + dx;
                    let y = origin_y + row as u32 * SCALE + dy;
                    if x < image.width() && y < image.height() {
                        image.put_pixel(x, y, INK);
                    }
                }
            }
        }
    }
}

fn glyph(ch: char) -> &'static [&'static str] {
    match ch {
        '0' => &[
            "01110", "10001", "10011", "10101", "11001", "10001", "01110",
        ],
        '1' => &[
            "00100", "01100", "00100", "00100", "00100", "00100", "01110",
        ],
        '2' => &[
            "01110", "10001", "00001", "00010", "00100", "01000", "11111",
        ],
        '3' => &[
            "01110", "10001", "00001", "00110", "00001", "10001", "01110",
        ],
        '4' => &[
            "00010", "00110", "01010", "10010", "11111", "00010", "00010",
        ],
        '5' => &[
            "11111", "10000", "11110", "00001", "00001", "10001", "01110",
        ],
        '6' => &[
            "00110", "01000", "10000", "11110", "10001", "10001", "01110",
        ],
        '7' => &[
            "11111", "00001", "00010", "00100", "01000", "01000", "01000",
        ],
        '8' => &[
            "01110", "10001", "10001", "01110", "10001", "10001", "01110",
        ],
        '9' => &[
            "01110", "10001", "10001", "01111", "00001", "00010", "01100",
        ],
        _ => &[
            "00000", "00100", "00100", "11111", "00100", "00100", "00000",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(waiting: usize) -> image::RgbaImage {
        let icon = status_icon(waiting);
        let image = image::load_from_memory(&icon.png).expect("png").to_rgba8();
        assert_eq!(icon.width_pt, image.width() as f32 / SCALE as f32);
        assert_eq!(icon.height_pt, image.height() as f32 / SCALE as f32);
        image
    }

    fn badge(pixel: &image::Rgba<u8>) -> bool {
        pixel[0] > 220 && pixel[1] < 90 && pixel[2] < 80 && pixel[3] > 200
    }

    #[test]
    fn waiting_badge_is_telegram_red_and_grows_with_the_count() {
        let plain = decode(0);
        let one = decode(3);
        let two = decode(12);
        let capped = decode(100);
        assert!(plain.width() < one.width());
        assert!(one.width() < two.width());
        assert!(two.width() < capped.width());
        assert_eq!(one.height(), plain.height());
        // The mark itself is red, so only pixels past the idle icon are the badge.
        let past = plain.width();
        let badge_red = one.pixels().enumerate().any(|(index, pixel)| {
            let x = index as u32 % one.width();
            let y = index as u32 / one.width();
            x >= past && y < 16 && badge(pixel)
        });
        assert!(badge_red, "badge extends past the icon in telegram red");
        let digit = one.pixels().enumerate().any(|(index, pixel)| {
            let x = index as u32 % one.width();
            let y = index as u32 / one.width();
            y < 16 && x + 12 > one.width() && pixel[0] > 230 && pixel[1] > 230 && pixel[2] > 230
        });
        assert!(digit, "badge count is white");
    }
}
