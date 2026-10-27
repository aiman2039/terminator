use eframe::egui;

use crate::{PaneSource, Tab};

/// Pane and top-level tab drag-and-drop state: the dragged item, where it
/// came from, the per-frame drop targets, and hover/preview feedback.
#[derive(Default)]
pub(crate) struct DragState {
    /// Strip tab whose drag has started. Promoted to [`Self::pane`] once
    /// the pointer leaves the strip, so reordering inside the strip stays
    /// with the dock.
    pub(crate) strip_tab: Option<String>,
    /// Terminal pane currently dragged by its caption header or, once the
    /// pointer leaves the IDE strip, by a strip tab. Dropped onto another
    /// split leaf, a workspace tab, or the opposite dock.
    pub(crate) pane: Option<Tab>,
    pub(crate) pane_source: Option<PaneSource>,
    /// Main-dock tab buttons painted this frame, as pane-drag drop targets.
    pub(crate) drop_tabs: Vec<(egui_dock::TabPath, egui::Rect)>,
    /// The dragged pane started in the IDE strip. A drop on the main pane
    /// moves it there; a drop back on the strip cancels that move.
    pub(crate) pane_from_strip: bool,
    /// Top-level tab currently dragged by its strip tab. Dropping it over
    /// the strip reorders it to the insertion slot; releasing elsewhere
    /// cancels. Only one of `pane` and `tab` is active at a time.
    pub(crate) tab: Option<String>,
    /// Last text snapshot of the dragged terminal, taken when its drag
    /// starts and shown in the floating ghost.
    pub(crate) pane_snapshot: Vec<String>,
    /// Group previewed while a pane drag hovers its strip tab, as
    /// (project, origin group) so a cancelled drag can switch back.
    pub(crate) preview_origin: Option<(String, String)>,
    /// A pane drag hovers the interior of a strip tab this frame. The dock
    /// paints the previewed tab's focused leaf at real size so the
    /// move-into outcome is visible, not just the strip outline.
    pub(crate) strip_tab_hover: bool,
    /// A pane drag hovers a strip gap, "+", or empty strip background this
    /// frame, where a release opens a fresh top-level tab. The tab ghost
    /// (not the pane snapshot ghost) follows the pointer there.
    pub(crate) strip_new_tab_hover: bool,
}
