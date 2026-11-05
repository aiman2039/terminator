//! The right end of a strip tab: the close glyph, the pin mark, or the dot
//! for unsaved changes.
use super::super::super::*;

/// Radius of the unsaved dot: about half the close glyph.
const MODIFIED_DOT_RADIUS: f32 = 4.0;

/// What a strip tab's close slot shows this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseSlot {
    Empty,
    Close,
    Pinned,
    /// Unsaved changes: a dot until the pointer reaches the slot, like the
    /// `JetBrains` New UI editor tabs.
    Modified,
}

impl CloseSlot {
    /// The pointer on the slot always shows the close glyph, so a click
    /// there closes as before. Otherwise unsaved changes win over the pin
    /// mark, and the glyph shows only on the active or hovered tab.
    pub(super) fn pick(modified: bool, pinned: bool, close_hovered: bool, tab_hot: bool) -> Self {
        if close_hovered {
            Self::Close
        } else if modified {
            Self::Modified
        } else if pinned {
            Self::Pinned
        } else if tab_hot {
            Self::Close
        } else {
            Self::Empty
        }
    }
}

impl App {
    pub(super) fn paint_close_slot(&self, ui: &egui::Ui, rect: egui::Rect, slot: CloseSlot) {
        let center = rect.center();
        match slot {
            CloseSlot::Empty => {}
            CloseSlot::Close => {
                egui::Image::new(icons::source("X"))
                    .tint(appearance::icon_color())
                    .paint_at(
                        ui,
                        egui::Rect::from_center_size(
                            center,
                            egui::Vec2::splat(appearance::TOOLBAR_ICON),
                        ),
                    );
            }
            CloseSlot::Pinned => {
                ui.painter().text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "◆",
                    egui::FontId::proportional(appearance::FONT_SMALL),
                    appearance::color(&self.theme.accent),
                );
            }
            CloseSlot::Modified => {
                ui.painter()
                    .circle_filled(center, MODIFIED_DOT_RADIUS, appearance::icon_color());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CloseSlot;

    #[test]
    fn unsaved_tabs_show_a_dot_until_the_close_slot_is_hovered() {
        assert_eq!(
            CloseSlot::pick(true, false, false, false),
            CloseSlot::Modified
        );
        assert_eq!(
            CloseSlot::pick(true, false, false, true),
            CloseSlot::Modified
        );
        assert_eq!(CloseSlot::pick(true, false, true, true), CloseSlot::Close);
    }

    #[test]
    fn the_unsaved_dot_wins_over_the_pin_mark() {
        assert_eq!(
            CloseSlot::pick(true, true, false, true),
            CloseSlot::Modified
        );
        assert_eq!(CloseSlot::pick(false, true, false, true), CloseSlot::Pinned);
        assert_eq!(CloseSlot::pick(false, true, true, true), CloseSlot::Close);
    }

    #[test]
    fn saved_tabs_show_the_close_glyph_only_when_active_or_hovered() {
        assert_eq!(CloseSlot::pick(false, false, false, true), CloseSlot::Close);
        assert_eq!(
            CloseSlot::pick(false, false, false, false),
            CloseSlot::Empty
        );
    }
}
