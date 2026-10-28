//! Workspace strip: the project's top-level tabs, their drag and drop,
//! and the new-tab and recent-tab controls.
mod drop;
mod face;
mod strip;
mod tab;

/// Strip actions collected while painting, applied after the strip.
#[derive(Default)]
struct StripActions {
    switch: Option<String>,
    close: Option<String>,
    close_all: bool,
    close_left: Option<String>,
    close_right: Option<String>,
    toggle_pin: Option<String>,
    add_at: Option<usize>,
}

/// Per-frame values shared by the strip steps.
#[derive(Clone, Copy)]
struct StripFrame<'a> {
    project: &'a str,
    strip_rect: eframe::egui::Rect,
    reveal: bool,
    /// A pane from this project is being dragged, so strip tabs are drop
    /// targets for it.
    drag_same_project: bool,
}
