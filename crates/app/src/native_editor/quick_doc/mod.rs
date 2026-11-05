//! Quick Documentation (`JetBrains`): one popup with the docs of the
//! symbol at a spot, under the problems there. A pointer that rests on a
//! word for [`REST`] opens it; F1, Ctrl+J, Cmd/Ctrl+K and vim `K` open it
//! at the caret. A key press, focus loss, or a click or scroll outside
//! the popup hides it. A pointer popup also hides when the pointer leaves
//! its word, unless the pointer moves into the popup (to scroll and copy).
pub(crate) mod popup;
#[cfg(test)]
mod tests;

use crate::lsp_manager::HoverKey;
use eframe::egui::{self, Pos2, Rect};
use std::time::{Duration, Instant};
use terminator_native_edit::lsp::Severity;
use terminator_native_edit::view::{HoverSpot, Squiggle, word_span};

/// How long the pointer rests on a word before its popup opens.
pub(crate) const REST: Duration = Duration::from_millis(500);
/// After a key press or a click, the pointer must move this far before a
/// new rest starts.
const STILL: f32 = 2.0;
/// Room around the popup that still counts as inside it.
const MARGIN: f32 = 4.0;

/// What a popup is about: a word (or one char) of one source view.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spot {
    /// The source view (split views of one file differ).
    pub(crate) view: String,
    pub(crate) line: usize,
    /// The char asked about.
    pub(crate) col: usize,
    /// Char columns of the word, or of the one char.
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) word: bool,
    /// Screen rect the popup goes under.
    pub(crate) anchor: Rect,
}

impl Spot {
    /// The spot the source view reported under the pointer.
    pub(crate) fn pointer(view: &str, hover: HoverSpot) -> Self {
        Self {
            view: view.to_owned(),
            line: hover.line,
            col: hover.col,
            start: hover.start,
            end: hover.end,
            word: hover.word,
            anchor: hover.rect,
        }
    }

    /// The same text of the same view (the pointer may move inside it).
    pub(crate) fn same(&self, other: &Spot) -> bool {
        self.view == other.view
            && (self.line, self.start, self.end) == (other.line, other.start, other.end)
    }

    /// The char to ask the server about: inside the word, so a caret just
    /// after a word asks about its last char.
    pub(crate) fn ask_col(&self) -> usize {
        if self.word {
            self.col
                .clamp(self.start, self.end.saturating_sub(1).max(self.start))
        } else {
            self.col
        }
    }
}

/// Who opened a popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A resting pointer: the popup follows the pointer.
    Pointer,
    /// F1, Ctrl+J, Cmd/Ctrl+K or `K`: the popup stays until dismissed.
    Caret,
}

/// One popup: on screen, or waiting for its docs.
pub(crate) struct Shown {
    pub(crate) spot: Spot,
    pub(crate) origin: Origin,
    /// The docs asked for, and since when.
    pub(crate) docs: Option<(HoverKey, Instant)>,
    /// Problems at the spot, worst first.
    pub(crate) problems: Vec<(Severity, String)>,
    /// Why there are no docs (no server, still starting).
    pub(crate) note: Option<String>,
    /// Where the popup was drawn last frame.
    pub(crate) rect: Option<Rect>,
    /// The frame pass its view last drew in.
    pub(crate) pass: u64,
}

/// One view's input for one frame.
pub(crate) struct Input<'a> {
    pub(crate) now: Instant,
    pub(crate) view: &'a str,
    /// The spot under the pointer in this view.
    pub(crate) spot: Option<Spot>,
    pub(crate) pointer: Option<Pos2>,
    /// A key was pressed, or text typed, pasted or cut.
    pub(crate) typed: bool,
    /// Where a pointer button went down, while it is down.
    pub(crate) pressed: Option<Pos2>,
    pub(crate) scrolled: bool,
    /// The app window has keyboard focus.
    pub(crate) focused: bool,
}

/// What a view does after [`QuickDoc::step`].
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    Idle,
    /// The pointer rests: look again after this wait.
    Wait(Duration),
    /// The pointer rested on this spot long enough: open its popup.
    Open(Spot),
}

/// The pointer timer and the popup of one file's views.
#[derive(Default)]
pub(crate) struct QuickDoc {
    /// The spot under the resting pointer, and since when.
    rest: Option<(Spot, Instant)>,
    /// The pointer position at the last key press or click.
    quiet: Option<Pos2>,
    pub(crate) shown: Option<Shown>,
}

impl QuickDoc {
    /// One frame of one view: hide the popup when the input says so, and
    /// time the rest of the pointer on this view's text.
    pub(crate) fn step(&mut self, input: Input<'_>) -> Step {
        if self.cancelled(&input) {
            self.rest = None;
            self.shown = None;
            self.quiet = input.pointer;
            return Step::Idle;
        }
        self.leave(&input);
        self.rest_on(input)
    }

    /// Whether `at` is in the popup.
    pub(crate) fn contains(&self, at: Pos2) -> bool {
        self.shown
            .as_ref()
            .and_then(|shown| shown.rect)
            .is_some_and(|rect| rect.expand(MARGIN).contains(at))
    }

    /// The popup of `view` is gone when that view did not draw in the
    /// pass before `pass` (its tab was hidden).
    pub(crate) fn drop_stale(&mut self, view: &str, pass: u64) {
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.spot.view == view && shown.pass.saturating_add(1) < pass)
        {
            self.shown = None;
        }
    }

    /// A key, focus loss, or a click or scroll away from the popup.
    fn cancelled(&self, input: &Input<'_>) -> bool {
        let outside = |at: Pos2| !self.contains(at);
        input.typed
            || !input.focused
            || input.pressed.is_some_and(outside)
            || (input.scrolled && input.pointer.is_none_or(outside))
    }

    /// A pointer popup of this view hides once the pointer is neither on
    /// its spot nor in the popup.
    fn leave(&mut self, input: &Input<'_>) {
        let Some(shown) = &self.shown else {
            return;
        };
        let on_spot = input
            .spot
            .as_ref()
            .is_some_and(|spot| spot.same(&shown.spot));
        let in_popup = input.pointer.is_some_and(|at| self.contains(at));
        if shown.origin == Origin::Pointer && shown.spot.view == input.view && !on_spot && !in_popup
        {
            self.shown = None;
        }
    }

    /// Time the rest on the spot under the pointer: a new spot starts the
    /// timer; after a key press or click the pointer must move first.
    fn rest_on(&mut self, input: Input<'_>) -> Step {
        let Some(spot) = input.spot else {
            if self
                .rest
                .as_ref()
                .is_some_and(|(rest, _)| rest.view == input.view)
            {
                self.rest = None;
            }
            return Step::Idle;
        };
        if let Some(quiet) = self.quiet {
            if input.pointer.is_some_and(|at| at.distance(quiet) < STILL) {
                return Step::Idle;
            }
            self.quiet = None;
        }
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.spot.same(&spot))
        {
            self.rest = None;
            return Step::Idle;
        }
        let since = match &self.rest {
            Some((rest, since)) if rest.same(&spot) => *since,
            _ => {
                self.rest = Some((spot.clone(), input.now));
                input.now
            }
        };
        let rested = input.now.saturating_duration_since(since);
        if rested < REST {
            return Step::Wait(REST.saturating_sub(rested));
        }
        self.rest = None;
        Step::Open(spot)
    }
}

/// Temp-data key: the frame pass a popup last drew in.
fn on_screen_key() -> egui::Id {
    egui::Id::unique("quick-doc-on-screen")
}

/// A popup drew this pass (any view, any file).
pub(crate) fn note_on_screen(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(on_screen_key(), pass));
}

/// Copy belongs to a popup while one is on screen and label text (the
/// popup's) is selected. Every editor view leaves Copy to it, not only the
/// view it belongs to: the pointer may have opened it over another pane.
pub(crate) fn popup_holds_copy(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(on_screen_key()))
        .is_some_and(|pass| pass.saturating_add(1) >= now)
        && ctx
            .with_plugin(|labels: &mut egui::text_selection::LabelSelectionState| {
                labels.has_selection()
            })
            .unwrap_or(false)
}

/// What a caret at char column `col` of `line` asks about: the identifier
/// that holds it or ends at it, else the one char under it. Returns its
/// char columns and whether it is a word.
pub(crate) fn caret_span(line: &str, col: usize) -> (usize, usize, bool) {
    let word = word_span(line, col).or_else(|| word_span(line, col.checked_sub(1)?));
    match word {
        Some((start, end)) => (start, end, true),
        None => (col, col.saturating_add(1), false),
    }
}

/// The problems underlined at `spot`, worst first, each message once.
pub(crate) fn problems_at(
    heads: &[Squiggle],
    tails: &[Squiggle],
    spot: &Spot,
) -> Vec<(Severity, String)> {
    let mut found: Vec<(Severity, String)> = Vec::new();
    for squiggle in heads.iter().chain(tails) {
        let covers =
            squiggle.line == spot.line && squiggle.start < spot.end && spot.start < squiggle.end;
        let problem = (squiggle.severity, squiggle.message.clone());
        if covers && !found.contains(&problem) {
            found.push(problem);
        }
    }
    found.sort_by_key(|(severity, _)| *severity);
    found
}
