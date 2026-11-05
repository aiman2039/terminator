//! Commit graph lane colors: one fixed palette for every theme. The colors
//! are mid-tones, so each keeps 3:1 contrast on light and dark surfaces.
use eframe::egui::Color32;

/// Neighboring entries differ in hue, because new lanes take the next one.
pub const LANE_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x4A, 0x8F, 0xE7), // blue
    Color32::from_rgb(0x2E, 0x9E, 0x4F), // green
    Color32::from_rgb(0xE0, 0x70, 0x0F), // orange
    Color32::from_rgb(0x90, 0x65, 0xE0), // purple
    Color32::from_rgb(0xD9, 0x47, 0x5A), // red
    Color32::from_rgb(0x13, 0x9A, 0xA8), // teal
    Color32::from_rgb(0xB3, 0x86, 0x00), // amber
    Color32::from_rgb(0xCC, 0x4F, 0xA0), // pink
];

/// The color of lane palette index `index`; indexes wrap.
pub fn lane_color(index: u8) -> Color32 {
    usize::from(index)
        .checked_rem(LANE_COLORS.len())
        .and_then(|index| LANE_COLORS.get(index))
        .copied()
        .unwrap_or(Color32::GRAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::appearance::AppearanceConfig;

    /// WCAG relative luminance.
    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let c = f32::from(value) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn lane_colors_read_on_every_preset_surface() {
        for theme in [
            AppearanceConfig::islands_dark(),
            AppearanceConfig::intellij_dark(),
            AppearanceConfig::terminator_dark(),
            AppearanceConfig::high_contrast(),
            AppearanceConfig::intellij_light(),
        ] {
            for background in [&theme.surface, &theme.window] {
                let background = crate::appearance::color(background);
                for lane in LANE_COLORS {
                    let ratio = contrast(lane, background);
                    assert!(ratio >= 3.0, "{lane:?} on {background:?}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn lane_color_indexes_wrap_around_the_palette() {
        assert_eq!(lane_color(0), LANE_COLORS[0]);
        assert_eq!(lane_color(8), LANE_COLORS[0]);
        assert_eq!(lane_color(255), LANE_COLORS[7]);
        assert_ne!(lane_color(0), lane_color(1));
    }
}
