//! State counters: the trailing "icon + count" marks on project, tab
//! group and worktree rows.
use super::super::scale::{FONT_SMALL, RADIUS_LARGE};
use super::{PaintStatusIconInput, paint_status_icon};
use eframe::egui::{self, Color32, FontId};

/// One state's count on a row: its icon and number, both in the state
/// color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateCounter {
    pub icon: &'static str,
    pub count: usize,
    pub color: Color32,
    /// A waiting counter stays when the row is narrow.
    pub keep: bool,
}

/// Optional parts of a sidebar row beyond its label and trailing text.
#[derive(Clone, Copy, Default)]
pub struct RowExtras<'a> {
    /// State counters, in place of the trailing text.
    pub counters: &'a [StateCounter],
    /// A second line in colored parts, in place of a plain subtitle.
    pub detail: &'a [(&'a str, Color32)],
    /// Trailing words keep the row tint, such as a session's "waiting 4m".
    pub tint_words: bool,
    /// How a selected row shows its selection.
    pub selection: RowSelection,
    /// A live-state dot at the row's right edge, in this color.
    pub status_dot: Option<Color32>,
    /// The trailing text is a count, painted as a small pill.
    pub count_pill: bool,
}

/// How a selected sidebar row shows its selection. Hover and keyboard
/// focus paint the same way for every kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowSelection {
    /// The selection fill.
    #[default]
    Fill,
    /// The selection fill and an accent bar on the left edge: the one
    /// current row of a tree (the active session).
    Bar,
    /// No fill: the caller paints stronger text (an open project).
    Text,
}

const ICON: f32 = 12.0;
const ICON_GAP: f32 = 2.0;
const COUNTER_GAP: f32 = 8.0;
const EDGE: f32 = 5.0;

fn count_width(ui: &egui::Ui, counter: &StateCounter) -> f32 {
    ui.painter()
        .layout_no_wrap(
            counter.count.to_string(),
            FontId::proportional(FONT_SMALL),
            counter.color,
        )
        .size()
        .x
}

fn counter_width(ui: &egui::Ui, counter: &StateCounter) -> f32 {
    ICON + ICON_GAP + count_width(ui, counter)
}

/// The counters that fit in `room`, in their order. Counters go from the
/// right when the row is narrow; a kept (waiting) counter never goes.
pub(super) fn fitted<'c>(
    ui: &egui::Ui,
    counters: &'c [StateCounter],
    room: f32,
) -> Vec<&'c StateCounter> {
    let mut shown: Vec<&StateCounter> = counters.iter().filter(|c| c.count > 0).collect();
    while shown.len() > 1 && total_width(ui, &shown) > room {
        let Some(drop) = shown.iter().rposition(|c| !c.keep) else {
            break;
        };
        shown.remove(drop);
    }
    shown
}

/// Width of `counters` with their gaps and the row edge; 0 for none.
pub(super) fn total_width(ui: &egui::Ui, counters: &[&StateCounter]) -> f32 {
    if counters.is_empty() {
        return 0.0;
    }
    let gaps = COUNTER_GAP
        * f32::from(u16::try_from(counters.len().saturating_sub(1)).unwrap_or(u16::MAX));
    counters
        .iter()
        .map(|counter| counter_width(ui, counter))
        .sum::<f32>()
        + gaps
        + EDGE
        + 4.0
}

/// Paint `counters` right-aligned at `anchor` (the row's right edge and
/// label center).
pub(super) fn paint(ui: &egui::Ui, anchor: egui::Pos2, counters: &[&StateCounter]) {
    let mut right = anchor.x - EDGE;
    for counter in counters.iter().rev() {
        let text = count_width(ui, counter);
        ui.painter().text(
            egui::pos2(right, anchor.y),
            egui::Align2::RIGHT_CENTER,
            counter.count.to_string(),
            FontId::proportional(FONT_SMALL),
            counter.color,
        );
        right -= text + ICON_GAP;
        paint_status_icon(PaintStatusIconInput {
            ui,
            rect: egui::Rect::from_min_max(
                egui::pos2(right - ICON, anchor.y - ICON * 0.5),
                egui::pos2(right, anchor.y + ICON * 0.5),
            ),
            icon: counter.icon,
            tint: counter.color,
            spin: false,
        });
        right -= ICON + COUNTER_GAP;
    }
}

const PILL_HEIGHT: f32 = 16.0;
const PILL_PAD: f32 = 6.0;

fn pill_galley(ui: &egui::Ui, text: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(FONT_SMALL), color)
}

/// Width of a count pill with the row edge.
pub(super) fn pill_width(ui: &egui::Ui, text: &str) -> f32 {
    pill_galley(ui, text, Color32::PLACEHOLDER).size().x + PILL_PAD * 2.0 + EDGE + 4.0
}

/// A count in a small rounded pill, right-aligned at `anchor` (the row's
/// right edge and label center). The fill is a faint tint of `color`, so
/// it reads on the panel and on a hovered or selected row.
pub(super) fn paint_pill(ui: &egui::Ui, anchor: egui::Pos2, text: &str, color: Color32) {
    let galley = pill_galley(ui, text, color);
    let width = (galley.size().x + PILL_PAD * 2.0).max(PILL_HEIGHT);
    let rect = egui::Rect::from_min_max(
        egui::pos2(anchor.x - EDGE - width, anchor.y - PILL_HEIGHT * 0.5),
        egui::pos2(anchor.x - EDGE, anchor.y + PILL_HEIGHT * 0.5),
    );
    ui.painter()
        .rect_filled(rect, RADIUS_LARGE, color.gamma_multiply(0.18));
    let size = galley.size();
    let center = rect.center();
    ui.painter().galley(
        egui::pos2(center.x - size.x * 0.5, center.y - size.y * 0.5),
        galley,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter(icon: &'static str, count: usize, keep: bool) -> StateCounter {
        StateCounter {
            icon,
            count,
            color: Color32::YELLOW,
            keep,
        }
    }

    #[test]
    fn a_narrow_row_drops_counters_from_the_right_but_keeps_waiting() {
        let ctx = egui::Context::default();
        let counters = [
            counter("ShieldQuestion", 1, true),
            counter("CircleAlert", 2, false),
            counter("LoaderCircle", 3, false),
        ];
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let icons =
                |shown: Vec<&StateCounter>| shown.iter().map(|c| c.icon).collect::<Vec<_>>();
            assert_eq!(icons(fitted(ui, &counters, 500.0)).len(), 3);
            let two = total_width(ui, &[&counters[0], &counters[1]]);
            assert_eq!(
                icons(fitted(ui, &counters, two)),
                vec!["ShieldQuestion", "CircleAlert"]
            );
            assert_eq!(icons(fitted(ui, &counters, 0.0)), vec!["ShieldQuestion"]);
            // Without a waiting counter the first one still stays.
            assert_eq!(icons(fitted(ui, &counters[1..], 0.0)), vec!["CircleAlert"]);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn zero_counts_are_not_shown() {
        let ctx = egui::Context::default();
        let counters = [
            counter("ShieldQuestion", 0, true),
            counter("LoaderCircle", 2, false),
        ];
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let shown = fitted(ui, &counters, 500.0);
            assert_eq!(shown.len(), 1);
            assert_eq!(shown[0].icon, "LoaderCircle");
        });
        output.textures_delta.clear();
    }
}
