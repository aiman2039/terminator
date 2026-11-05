//! Quick Documentation timing and placement: the rest before a popup
//! opens, what hides it again, and the problems it lists.
use super::popup::{MAX_HEIGHT, place};
use super::*;
use eframe::egui::{Align2, pos2, vec2};

const VIEW: &str = "left";

/// Chars `start..end` of line 3 in `view`, drawn 8 px per char.
fn spot(view: &str, (start, end): (u16, u16)) -> Spot {
    Spot {
        view: view.to_owned(),
        line: 3,
        col: usize::from(start),
        start: usize::from(start),
        end: usize::from(end),
        word: true,
        anchor: Rect::from_min_max(
            pos2(f32::from(start) * 8.0, 40.0),
            pos2(f32::from(end) * 8.0, 56.0),
        ),
    }
}

/// A popup for `spot`, drawn under it.
fn shown(spot: Spot, origin: Origin) -> Shown {
    let rect = Rect::from_min_size(spot.anchor.left_bottom(), vec2(200.0, 100.0));
    Shown {
        spot,
        origin,
        docs: None,
        problems: Vec::new(),
        note: None,
        rect: Some(rect),
        pass: 1,
    }
}

/// A quiet frame of [`VIEW`] at `now` with the pointer on `on`.
fn still(now: Instant, on: Option<&Spot>) -> Input<'static> {
    Input {
        now,
        view: VIEW,
        spot: on.cloned(),
        pointer: on.map(|spot| spot.anchor.center()),
        typed: false,
        pressed: None,
        scrolled: false,
        focused: true,
    }
}

fn after(start: Instant, ms: u64) -> Instant {
    start
        .checked_add(Duration::from_millis(ms))
        .expect("a later instant")
}

/// A point inside the popup [`shown`] draws for `spot`.
fn in_popup(spot: &Spot) -> Pos2 {
    pos2(spot.anchor.left() + 20.0, spot.anchor.bottom() + 30.0)
}

#[test]
fn a_popup_opens_once_the_pointer_rests_on_a_word_for_the_delay() {
    let mut quick = QuickDoc::default();
    let word = spot(VIEW, (4, 9));
    let start = Instant::now();
    assert_eq!(quick.step(still(start, Some(&word))), Step::Wait(REST));
    // Another char of the same word keeps the rest going.
    let mut inside = word.clone();
    inside.col = 7;
    assert_eq!(
        quick.step(still(after(start, 300), Some(&inside))),
        Step::Wait(Duration::from_millis(200))
    );
    assert_eq!(
        quick.step(still(after(start, 500), Some(&word))),
        Step::Open(word.clone())
    );
    // Another word starts a new rest.
    let other = spot(VIEW, (12, 15));
    assert_eq!(
        quick.step(still(after(start, 600), Some(&other))),
        Step::Wait(REST)
    );
    assert_eq!(quick.step(still(after(start, 700), None)), Step::Idle);
    assert_eq!(
        quick.step(still(after(start, 800), Some(&other))),
        Step::Wait(REST),
        "leaving the text ends the rest"
    );
}

#[test]
fn a_pointer_popup_stays_on_its_word_and_in_itself_and_hides_elsewhere() {
    let mut quick = QuickDoc::default();
    let word = spot(VIEW, (4, 9));
    quick.shown = Some(shown(word.clone(), Origin::Pointer));
    let now = Instant::now();
    assert_eq!(quick.step(still(now, Some(&word))), Step::Idle);
    assert!(quick.shown.is_some());
    // The view reports no word under the popup.
    let inside = Input {
        pointer: Some(in_popup(&word)),
        ..still(now, None)
    };
    assert_eq!(quick.step(inside), Step::Idle);
    assert!(quick.shown.is_some(), "the popup holds the pointer");
    // Another view's frame leaves it alone.
    let other_view = Input {
        view: "right",
        pointer: Some(pos2(500.0, 500.0)),
        ..still(now, None)
    };
    quick.step(other_view);
    assert!(quick.shown.is_some());
    let away = Input {
        pointer: Some(pos2(500.0, 500.0)),
        ..still(now, None)
    };
    quick.step(away);
    assert!(
        quick.shown.is_none(),
        "neither on the word nor in the popup"
    );
}

#[test]
fn a_key_hides_the_popup_and_no_rest_starts_until_the_pointer_moves() {
    let mut quick = QuickDoc::default();
    let word = spot(VIEW, (4, 9));
    quick.shown = Some(shown(word.clone(), Origin::Pointer));
    let start = Instant::now();
    let typed = Input {
        typed: true,
        ..still(start, Some(&word))
    };
    assert_eq!(quick.step(typed), Step::Idle);
    assert!(quick.shown.is_none());
    assert_eq!(
        quick.step(still(after(start, 900), Some(&word))),
        Step::Idle
    );
    let center = word.anchor.center();
    let moved = Input {
        pointer: Some(pos2(center.x + 3.0, center.y)),
        ..still(after(start, 950), Some(&word))
    };
    assert_eq!(quick.step(moved), Step::Wait(REST));
}

#[test]
fn a_caret_popup_stays_while_the_pointer_wanders_until_a_click_elsewhere() {
    let mut quick = QuickDoc::default();
    let word = spot(VIEW, (4, 9));
    quick.shown = Some(shown(word.clone(), Origin::Caret));
    let now = Instant::now();
    let far = pos2(500.0, 500.0);
    quick.step(Input {
        pointer: Some(far),
        ..still(now, None)
    });
    assert!(quick.shown.is_some(), "F1 popups ignore the pointer");
    // Pressing in the popup (to select its text) keeps it.
    quick.step(Input {
        pointer: Some(in_popup(&word)),
        pressed: Some(in_popup(&word)),
        ..still(now, None)
    });
    assert!(quick.shown.is_some());
    quick.step(Input {
        pointer: Some(far),
        pressed: Some(far),
        ..still(now, None)
    });
    assert!(quick.shown.is_none());
}

#[test]
fn scrolling_the_docs_keeps_the_popup_but_scrolling_the_editor_or_focus_loss_hides_it() {
    let mut quick = QuickDoc::default();
    let word = spot(VIEW, (4, 9));
    quick.shown = Some(shown(word.clone(), Origin::Caret));
    let now = Instant::now();
    quick.step(Input {
        pointer: Some(in_popup(&word)),
        scrolled: true,
        ..still(now, None)
    });
    assert!(quick.shown.is_some());
    quick.step(Input {
        pointer: Some(pos2(500.0, 10.0)),
        scrolled: true,
        ..still(now, None)
    });
    assert!(quick.shown.is_none());
    quick.shown = Some(shown(word, Origin::Caret));
    quick.step(Input {
        focused: false,
        ..still(now, None)
    });
    assert!(quick.shown.is_none(), "the window lost focus");
}

#[test]
fn a_popup_of_a_view_that_stopped_drawing_is_dropped() {
    let mut quick = QuickDoc {
        shown: Some(Shown {
            pass: 10,
            ..shown(spot(VIEW, (4, 9)), Origin::Caret)
        }),
        ..QuickDoc::default()
    };
    quick.drop_stale("right", 20);
    assert!(quick.shown.is_some(), "another view's popup");
    quick.drop_stale(VIEW, 11);
    assert!(quick.shown.is_some(), "drawn in the pass before");
    quick.drop_stale(VIEW, 12);
    assert!(quick.shown.is_none(), "its tab was hidden");
}

#[test]
fn every_view_leaves_copy_to_a_popup_on_screen_with_selected_text() {
    let ctx = egui::Context::default();
    // A selectable docs label, and (`popup`) a popup of any view drawn.
    let frame = |events: Vec<egui::Event>, popup: bool| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.label("Docs text from the popup");
            if popup {
                note_on_screen(ui.ctx());
            }
        });
        output.textures_delta.clear();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (from, to) = (pos2(2.0, 6.0), pos2(80.0, 6.0));
    frame(Vec::new(), true);
    frame(
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
    );
    frame(vec![egui::Event::PointerMoved(to)], true);
    frame(vec![button(to, false)], true);
    assert!(popup_holds_copy(&ctx), "the selection is the popup's");
    // Once no popup draws, the editors take Copy back.
    frame(Vec::new(), false);
    frame(Vec::new(), false);
    assert!(!popup_holds_copy(&ctx));
}

#[test]
fn the_caret_asks_about_the_word_it_is_in_or_just_after() {
    let line = "let value = other;";
    assert_eq!(caret_span(line, 6), (4, 9, true));
    assert_eq!(caret_span(line, 0), (0, 3, true));
    assert_eq!(caret_span(line, 9), (4, 9, true), "just after the word");
    assert_eq!(caret_span(line, 10), (10, 11, false), "punctuation");
    assert_eq!(caret_span(line, 18), (18, 19, false), "the line end");
    assert_eq!(caret_span("", 0), (0, 1, false), "an empty line");
}

#[test]
fn the_server_is_asked_about_a_char_inside_the_word() {
    let mut word = spot(VIEW, (4, 9));
    word.col = 9;
    assert_eq!(word.ask_col(), 8, "a caret just after the word");
    word.col = 2;
    assert_eq!(word.ask_col(), 4);
    let mut one_char = spot(VIEW, (12, 13));
    one_char.word = false;
    one_char.col = 12;
    assert_eq!(one_char.ask_col(), 12);
}

fn squiggle(line: usize, (start, end): (usize, usize), severity: Severity, text: &str) -> Squiggle {
    Squiggle {
        line,
        start,
        end,
        severity,
        message: text.to_owned(),
    }
}

#[test]
fn problems_under_the_spot_come_worst_first_and_once() {
    let word = spot(VIEW, (4, 9));
    let heads = vec![
        squiggle(3, (0, 2), Severity::Error, "elsewhere on the line"),
        squiggle(3, (6, 12), Severity::Warning, "unused"),
        squiggle(3, (8, 9), Severity::Error, "mismatched types"),
        squiggle(2, (4, 9), Severity::Error, "another line"),
    ];
    let tails = vec![squiggle(3, (0, 20), Severity::Warning, "unused")];
    assert_eq!(
        problems_at(&heads, &tails, &word),
        vec![
            (Severity::Error, "mismatched types".to_owned()),
            (Severity::Warning, "unused".to_owned()),
        ]
    );
}

#[test]
fn the_popup_goes_under_its_word_unless_the_room_above_is_larger() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let high = Rect::from_min_max(pos2(10.0, 40.0), pos2(50.0, 56.0));
    assert_eq!(
        place(high, screen),
        (high.left_bottom(), Align2::LEFT_TOP, MAX_HEIGHT)
    );
    let low = Rect::from_min_max(pos2(10.0, 560.0), pos2(50.0, 576.0));
    assert_eq!(
        place(low, screen),
        (low.left_top(), Align2::LEFT_BOTTOM, MAX_HEIGHT)
    );
    // Little room either way: the body still gets a usable height.
    let tiny = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 60.0));
    let middle = Rect::from_min_max(pos2(10.0, 22.0), pos2(50.0, 38.0));
    let (_, _, height) = place(middle, tiny);
    assert!(height > 0.0 && height <= MAX_HEIGHT, "{height}");
}
