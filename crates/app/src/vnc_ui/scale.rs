//! How the remote screen is sized in its pane.
use eframe::egui;

/// Scale modes of the VNC toolbar menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VncScale {
    /// Whole screen inside the pane.
    #[default]
    Fit,
    /// Pane width; scroll vertically.
    FitWidth,
    /// One remote pixel per screen pixel.
    Actual,
    /// Two screen pixels per remote pixel.
    Double,
}

impl VncScale {
    pub(crate) const ALL: [Self; 4] = [Self::Fit, Self::FitWidth, Self::Actual, Self::Double];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit",
            Self::FitWidth => "Fit width",
            Self::Actual => "100%",
            Self::Double => "200%",
        }
    }

    /// The remote screen fills the pane with no scroll bars, so the wheel
    /// goes to the remote instead of scrolling.
    pub(crate) fn fills_pane(self) -> bool {
        self == Self::Fit
    }

    /// Smooth sampling when scaled to the pane; exact pixels at fixed
    /// zoom so text stays sharp.
    pub(crate) fn filter(self) -> egui::TextureOptions {
        match self {
            Self::Fit | Self::FitWidth => egui::TextureOptions::LINEAR,
            Self::Actual | Self::Double => egui::TextureOptions::NEAREST,
        }
    }

    /// Painted size of a `tex_size` screen in an `available` pane.
    pub(crate) fn display_size(self, available: egui::Vec2, tex_size: egui::Vec2) -> egui::Vec2 {
        if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
            return tex_size;
        }
        let factor = match self {
            Self::Fit => (available.x / tex_size.x)
                .min(available.y / tex_size.y)
                .max(0.05),
            Self::FitWidth => (available.x / tex_size.x).max(0.05),
            Self::Actual => 1.0,
            Self::Double => 2.0,
        };
        egui::vec2(tex_size.x * factor, tex_size.y * factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scale_sizes_the_screen_for_its_mode() {
        let pane = egui::vec2(800.0, 450.0);
        let screen = egui::vec2(1600.0, 1800.0);
        assert_eq!(
            VncScale::Fit.display_size(pane, screen),
            egui::vec2(400.0, 450.0)
        );
        assert_eq!(
            VncScale::FitWidth.display_size(pane, screen),
            egui::vec2(800.0, 900.0)
        );
        assert_eq!(VncScale::Actual.display_size(pane, screen), screen);
        assert_eq!(
            VncScale::Double.display_size(pane, screen),
            egui::vec2(3200.0, 3600.0)
        );
        assert!(VncScale::Fit.fills_pane() && !VncScale::FitWidth.fills_pane());
        assert_eq!(VncScale::Actual.filter(), egui::TextureOptions::NEAREST);
        assert_eq!(VncScale::FitWidth.filter(), egui::TextureOptions::LINEAR);
    }
}
