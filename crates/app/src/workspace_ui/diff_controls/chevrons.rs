//! JetBrains-style hunk action chevrons in the band between split panes.
use super::super::super::*;
use super::{ControlsFrame, ControlsState};
use diff::actions::Kind;

/// Which way a chevron moves a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chevron {
    /// `>>` at the left edge: make the right side like the left side.
    Revert,
    /// `<<` at the right edge: make the left side like the right side.
    Apply,
}

/// Chevrons of one hunk and the action each runs. Working tree (Index →
/// Working tree): `>>` discards the change (after confirmation) and `<<`
/// stages it. Staged (HEAD → Index): `>>` unstages it.
pub(crate) fn chevron_actions(staged: bool) -> &'static [(Chevron, Kind)] {
    if staged {
        &[(Chevron::Revert, Kind::Unstage)]
    } else {
        &[
            (Chevron::Revert, Kind::Discard),
            (Chevron::Apply, Kind::Stage),
        ]
    }
}

/// Fixture name of a hunk action, shared with the unified gutter buttons.
#[cfg(feature = "test-support")]
pub(super) fn action_target(kind: Kind, index: usize) -> String {
    if kind == Kind::Discard {
        format!("diff-hunk-discard-{index}")
    } else {
        format!("diff-hunk-action-{index}")
    }
}

/// Tooltip of a hunk action: the picked lines, or the whole hunk.
pub(super) fn action_tip(kind: Kind, picked: usize) -> String {
    let target = if picked > 0 {
        format!(
            "{picked} selected line{}",
            if picked == 1 { "" } else { "s" }
        )
    } else {
        "this change (Alt+click lines to pick some)".to_owned()
    };
    if kind == Kind::Discard {
        format!("Revert: discard {target}…")
    } else {
        format!("{} {target}", kind.label())
    }
}

/// Where one hunk's chevrons sit this frame.
pub(super) struct ChevronBand {
    pub(super) index: usize,
    pub(super) color: Color32,
    /// The hunk's old-side rows in the first pane.
    pub(super) left: egui::Rect,
    /// The hunk's new-side rows in the last pane.
    pub(super) right: egui::Rect,
}

impl ControlsFrame<'_> {
    /// The band between the panes, when this is a split view.
    pub(super) fn chevron_strip(&self) -> Option<egui::Rect> {
        if !self.split {
            return None;
        }
        let strip = egui::Rect::from_min_max(
            egui::pos2(self.first.inner_rect.right(), self.first.inner_rect.top()),
            egui::pos2(self.last.inner_rect.left(), self.last.inner_rect.bottom()),
        );
        (strip.width() >= 2.0 * CHEVRON_SIZE).then_some(strip)
    }

    /// Always-visible action chevrons of one hunk in the connector band.
    pub(super) fn paint_chevrons(
        &self,
        ui: &mut egui::Ui,
        band: &ChevronBand,
        state: &mut ControlsState<'_>,
    ) {
        let Some(strip) = self.chevron_strip() else {
            return;
        };
        let picked = state
            .review
            .picked
            .as_ref()
            .filter(|(hunk, _)| *hunk == band.index)
            .map_or(0, |(_, lines)| lines.len());
        for &(chevron, kind) in chevron_actions(self.staged) {
            let (x, side) = match chevron {
                Chevron::Revert => (strip.left() + 1.0, band.left),
                Chevron::Apply => (strip.right() - CHEVRON_SIZE - 1.0, band.right),
            };
            // Stick to the top of the visible part of a tall hunk.
            let last_row = side.bottom().max(side.top() + self.row_h) - self.row_h;
            let top = side.top().max(strip.top().min(last_row));
            let y = top + ((self.row_h - CHEVRON_SIZE) / 2.0).max(0.0);
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::Vec2::splat(CHEVRON_SIZE));
            if !strip.contains_rect(rect) {
                continue;
            }
            let response = ui
                .interact(
                    rect,
                    ui.scope_id()
                        .with(("diff-chevron", band.index, kind.label())),
                    if self.enabled {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                )
                .on_hover_text(action_tip(kind, picked))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &action_target(kind, band.index), rect);
            paint_chevron(ChevronPaint {
                ui,
                rect,
                chevron,
                color: band.color,
                tints: self.colors.tints(),
                hovered: response.hovered() && self.enabled,
                enabled: self.enabled,
            });
            if response.clicked() {
                state.result.action = Some((band.index, kind));
            }
        }
    }
}

/// Square side of one chevron button: two fit beside the divider handle.
const CHEVRON_SIZE: f32 = 12.0;

struct ChevronPaint<'a> {
    ui: &'a egui::Ui,
    rect: egui::Rect,
    chevron: Chevron,
    color: Color32,
    tints: appearance::DiffTints,
    hovered: bool,
    enabled: bool,
}

/// A crisp double chevron (`>>` or `<<`) on the pixel grid.
fn paint_chevron(paint: ChevronPaint<'_>) {
    let ChevronPaint {
        ui,
        rect,
        chevron,
        color,
        tints,
        hovered,
        enabled,
    } = paint;
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 2.0, super::super::diff_paint::tint(color, tints.edge));
    }
    let stroke_color = if enabled {
        color
    } else {
        color.gamma_multiply(0.4)
    };
    let stroke = egui::Stroke::new(1.5, stroke_color);
    let center = super::super::diff_paint::snap(ui, rect.center());
    let half = rect.height() * 0.22;
    let step = rect.width() * 0.24;
    let direction = match chevron {
        Chevron::Revert => 1.0,
        Chevron::Apply => -1.0,
    };
    for offset in [-0.5_f32, 0.5] {
        let tip_x = center.x + direction * (offset * step + half * 0.5);
        let back_x = tip_x - direction * half;
        painter.line(
            vec![
                egui::pos2(back_x, center.y - half),
                egui::pos2(tip_x, center.y),
                egui::pos2(back_x, center.y + half),
            ],
            stroke,
        );
    }
}
