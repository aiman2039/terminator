//! The Quick Documentation popup: the problems at its spot on top, then
//! the docs, under the spot, or above it when the screen has more room
//! there. Long docs scroll.
use super::Spot;
use crate::appearance;
use eframe::egui::{self, Align2, Color32, Pos2, Rect};
use terminator_native_edit::lsp::{Hover, Severity};

/// Width and height limits, as the `IntelliJ` documentation popup.
const MAX_WIDTH: f32 = 520.0;
pub(super) const MAX_HEIGHT: f32 = 360.0;
/// The popup frame's margin and border, outside the scrolled body.
const CHROME: f32 = 12.0;
/// The smallest body height, when the screen has little room.
const MIN_HEIGHT: f32 = 60.0;

/// What the popup shows.
pub(crate) struct Body<'a> {
    pub(crate) problems: &'a [(Severity, String)],
    /// Theme colors for errors and warnings.
    pub(crate) colors: (Color32, Color32),
    pub(crate) docs: Option<&'a Hover>,
    /// Shown instead of docs: why there are none.
    pub(crate) note: Option<&'a str>,
}

impl Body<'_> {
    pub(crate) fn is_empty(&self) -> bool {
        self.problems.is_empty() && self.docs.is_none() && self.note.is_none()
    }
}

/// Where the popup goes: under `anchor`, or above it when the room below
/// is too small and the room above is larger. Returns the point, the
/// popup corner placed on it, and the height the body may take.
pub(crate) fn place(anchor: Rect, screen: Rect) -> (Pos2, Align2, f32) {
    let below = screen.bottom() - anchor.bottom();
    let above = anchor.top() - screen.top();
    let (at, pivot, room) = if below >= MAX_HEIGHT + CHROME || below >= above {
        (anchor.left_bottom(), Align2::LEFT_TOP, below)
    } else {
        (anchor.left_top(), Align2::LEFT_BOTTOM, above)
    };
    (at, pivot, (room - CHROME).clamp(MIN_HEIGHT, MAX_HEIGHT))
}

/// Draw the popup for `spot`; returns its rect. `render` draws the docs
/// (the Markdown renderer of the completion docs panel).
pub(crate) fn show(
    ui: &egui::Ui,
    spot: &Spot,
    body: Body<'_>,
    render: impl FnOnce(&mut egui::Ui, &Hover),
) -> Rect {
    let (at, pivot, max_height) = place(spot.anchor, ui.ctx().content_rect());
    // One area per spot: a new popup measures itself afresh.
    let id = egui::Id::unique(("native-quick-doc", &spot.view, spot.line, spot.start));
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(at)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            appearance::popup_text_style(ui.style_mut());
            appearance::popup_frame(ui.style()).show(ui, |ui| {
                ui.set_max_width(MAX_WIDTH);
                egui::ScrollArea::vertical()
                    .id_salt(id)
                    .max_height(max_height)
                    .show(ui, |ui| body_ui(ui, body, render));
            });
        })
        .response
        .rect
}

/// The problems (errors and warnings in their theme colors), a rule,
/// then the docs or the note.
fn body_ui(ui: &mut egui::Ui, body: Body<'_>, render: impl FnOnce(&mut egui::Ui, &Hover)) {
    let (error, warning) = body.colors;
    for (severity, message) in body.problems {
        let text = egui::RichText::new(message);
        let text = match severity {
            Severity::Error => text.color(error),
            Severity::Warning => text.color(warning),
            _ => text,
        };
        ui.add(egui::Label::new(text).wrap());
    }
    if !body.problems.is_empty() && (body.docs.is_some() || body.note.is_some()) {
        ui.separator();
    }
    if let Some(docs) = body.docs {
        render(ui, docs);
    } else if let Some(note) = body.note {
        ui.weak(note);
    }
}
