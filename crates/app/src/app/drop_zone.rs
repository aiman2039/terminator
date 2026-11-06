//! Where a dragged terminal lands in a hovered split leaf.
use eframe::egui;

/// Drop zone within a hovered split leaf while a terminal is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneDropZone {
    /// The middle: swap single panes or join the leaf.
    Center,
    /// An edge band: open the dragged pane in a new split beside the leaf.
    Above,
    Below,
    Left,
    Right,
}
impl PaneDropZone {
    pub(crate) fn split(self) -> Option<egui_dock::Split> {
        match self {
            Self::Center => None,
            Self::Above => Some(egui_dock::Split::Above),
            Self::Below => Some(egui_dock::Split::Below),
            Self::Left => Some(egui_dock::Split::Left),
            Self::Right => Some(egui_dock::Split::Right),
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Center => "Place terminal here",
            Self::Above => "Split above",
            Self::Below => "Split below",
            Self::Left => "Split left",
            Self::Right => "Split right",
        }
    }
    /// The part of the leaf the dragged pane will occupy.
    pub(crate) fn landing(self, rect: egui::Rect) -> egui::Rect {
        let center = rect.center();
        match self {
            Self::Center => rect,
            Self::Above => egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), center.y)),
            Self::Below => egui::Rect::from_min_max(egui::pos2(rect.left(), center.y), rect.max),
            Self::Left => egui::Rect::from_min_max(rect.min, egui::pos2(center.x, rect.bottom())),
            Self::Right => egui::Rect::from_min_max(egui::pos2(center.x, rect.top()), rect.max),
        }
    }
}
