//! A strip tab's attention count and its active-tab accent underline.
use super::super::super::super::*;
use super::TabPaint;

impl App {
    /// Aggregate attention across the tab's terminals.
    pub(super) fn paint_strip_tab_badge(&self, ui: &egui::Ui, paint: &TabPaint<'_>) {
        let TabPaint {
            badge,
            rect,
            attention,
            ..
        } = paint;
        let Some((count, _)) = badge else {
            return;
        };
        let badge_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - 44.0, rect.center().y - 8.0),
            egui::pos2(rect.right() - 26.0, rect.center().y + 8.0),
        );
        ui.painter().text(
            badge_rect.right_center(),
            egui::Align2::RIGHT_CENTER,
            count,
            egui::FontId::proportional(appearance::FONT_CAPTION),
            appearance::color(if attention.waiting() > 0 {
                &self.theme.status_waiting
            } else {
                &self.theme.status_failed
            }),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("workspace-tab-attention:{}", paint.face.label),
            badge_rect,
        );
    }

    /// The accent underline: the mark of the active tab only.
    pub(super) fn paint_strip_tab_underline(&self, ui: &egui::Ui, paint: &TabPaint<'_>) {
        if !paint.active {
            return;
        }
        let rect = paint.rect;
        // Inset from the bottom edge. A stroke centered on rect.bottom() is
        // clipped away by the strip.
        let underline = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
        );
        ui.painter().rect_filled(
            underline,
            appearance::RADIUS_XSMALL,
            appearance::color(&self.theme.accent),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("workspace-tab-underline:{}", paint.face.label),
            underline,
        );
    }
}
