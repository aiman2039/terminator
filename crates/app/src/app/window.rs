use eframe::egui::{self};
use std::time::Instant;

use super::super::*;
pub(crate) fn begin_native_window_gesture(ctx: &egui::Context, command: egui::ViewportCommand) {
    ctx.send_viewport_cmd(command);
    // The window manager grabs pointer input and may consume mouse-up. Clear
    // egui's drag state at the handoff so the next control/edge can be pressed.
    ctx.stop_dragging();
    ctx.input_mut(|input| input.pointer = egui::PointerState::default());
}
pub(crate) fn window_resize_edges(ui: &mut egui::Ui) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ui.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let r = ui.ctx().content_rect();
    let edge = 4.0;
    let corner = 8.0;
    let regions = [
        (
            egui::Rect::from_min_max(r.min, egui::pos2(r.min.x + corner, r.min.y + corner)),
            D::NorthWest,
            C::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.right() - corner, r.top()),
                egui::pos2(r.right(), r.top() + corner),
            ),
            D::NorthEast,
            C::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left(), r.bottom() - corner),
                egui::pos2(r.left() + corner, r.bottom()),
            ),
            D::SouthWest,
            C::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(r.right() - corner, r.bottom() - corner), r.max),
            D::SouthEast,
            C::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left() + corner, r.top()),
                egui::pos2(r.right() - corner, r.top() + edge),
            ),
            D::North,
            C::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left() + corner, r.bottom() - edge),
                egui::pos2(r.right() - corner, r.bottom()),
            ),
            D::South,
            C::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left(), r.top() + corner),
                egui::pos2(r.left() + edge, r.bottom() - corner),
            ),
            D::West,
            C::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.right() - edge, r.top() + corner),
                egui::pos2(r.right(), r.bottom() - corner),
            ),
            D::East,
            C::ResizeHorizontal,
        ),
    ];
    // Panels own their full rectangles. Put only the narrow resize hit regions
    // above them, so the status bar cannot swallow edge drags.
    for (index, (rect, direction, cursor)) in regions.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("window-resize", index)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .movable(false)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::drag());
                let response = response.on_hover_cursor(cursor);
                #[cfg(feature = "test-support")]
                {
                    diagnostics::record(ui.ctx(), &format!("window-resize-{index}"), response.rect);
                    if std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_some()
                        && response.hovered()
                        && ui.input(|i| i.pointer.any_down())
                    {
                        eprintln!(
                            "Fixture resize input {index}: started={} dragged={}",
                            response.drag_started(),
                            response.dragged()
                        );
                    }
                }
                if response.drag_started() {
                    begin_native_window_gesture(
                        ui.ctx(),
                        egui::ViewportCommand::BeginResize(direction),
                    );
                }
            });
    }
}
pub(crate) fn header_drag_space(ui: &mut egui::Ui) {
    let response = ui.allocate_response(
        egui::vec2(ui.available_width().max(0.0), 30.0),
        egui::Sense::drag(),
    );
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "header-drag", response.rect);
    if response.drag_started() {
        begin_native_window_gesture(ui.ctx(), egui::ViewportCommand::StartDrag);
    }
}
/// Cheap fingerprint of the checked-out dock backing `pane_by_tab`/`pane_tabs`.
// Tab keys cost a JSON serialization each, so the maps are only rebuilt when
// this changes instead of every frame. Same-count cross-pane moves slip
// through and heal on the next structural change; readers only use the maps
// while the dock is checked out of `layouts`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PaneIndex {
    pub(crate) project: String,
    pub(crate) group: String,
    pub(crate) tabs: usize,
    pub(crate) focus: Option<egui_dock::NodeIndex>,
}
/// Live in-terminal find state for one session. Matches reference live grid
/// coordinates and go stale as output streams; the view recomputes them on a
/// throttle (see `terminal_view`).
#[derive(Default)]
pub struct TerminalFind {
    pub query: String,
    pub case_insensitive: bool,
    pub outcome: egui_term::FindOutcome,
    pub current: usize,
    pub searched_query: String,
    pub searched_case: bool,
    pub last_search: Option<Instant>,
}

impl TerminalFind {
    pub fn dirty(&self) -> bool {
        self.query != self.searched_query || self.case_insensitive != self.searched_case
    }

    pub fn step(&mut self, delta: isize) {
        let n = self.outcome.matches.len();
        if n == 0 {
            self.current = 0;
            return;
        }
        self.current = self
            .current
            .cast_signed()
            .saturating_add(delta)
            .rem_euclid(n.cast_signed())
            .cast_unsigned();
    }
}

/// A main-dock pane detached into its own OS window. The daemon keeps
/// owning the session; the window dies with the GUI like every other
/// GUI-only surface. `tab` is `None` only while the viewport renders it.
pub(crate) struct FloatingPane {
    pub(crate) viewport: egui::ViewportId,
    pub(crate) tab: Option<Tab>,
    pub(crate) home: (String, String),
}
