//! The gutter popup's Stage waits for a save and for a commit running in
//! the file's repository, and says why on hover.
use super::gutter::{GutterPopup, PopupAction, PopupUiInput, hunks, popup_ui};
use super::*;
use crate::app::painted_text;

/// One frame of the first change's popup; returns the picked action and
/// the painted texts.
fn popup_frame(
    ctx: &egui::Context,
    (can_stage, commit_lock): (bool, bool),
    (time, events): (f64, Vec<egui::Event>),
) -> (PopupAction, Vec<(egui::Pos2, String)>) {
    let found = hunks("a\nb\n", "a\nB\n");
    let mut popup = Some(GutterPopup {
        path: "/repo/a.rs".into(),
        index: 0,
        at: egui::pos2(10.0, 10.0),
    });
    let mut action = PopupAction::None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(700.0, 300.0),
        )),
        time: Some(time),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        action = popup_ui(PopupUiInput {
            ui,
            popup: &mut popup,
            hunks: &found,
            deleted: Color32::RED,
            can_stage,
            commit_lock,
        });
    });
    output.textures_delta.clear();
    (action, painted_text(&output.shapes))
}

#[test]
fn the_head_read_keeps_the_repository_root_that_the_gutter_stage_writes_in() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
    let path = PathBuf::from("/repo/src/a.rs");
    app.git_base_pending.insert(path.clone());
    let read = (Some("base\n".to_owned()), Some(PathBuf::from("/repo")));
    app.apply_git_base(path.clone(), 3, read);
    assert_eq!(app.gutter_root(&path), Some(Path::new("/repo")));
    assert!(app.git_base_pending.is_empty());
    // Outside Git there is no base and no root.
    app.apply_git_base(path.clone(), 4, (None, None));
    assert_eq!(app.gutter_root(&path), None);
}

#[test]
fn a_running_commit_disables_the_gutter_stage_and_says_why() {
    let wait = crate::workspace_ops::COMMIT_WAIT;
    for (state, why) in [
        ((true, true), Some(wait)),
        ((false, true), Some(wait)),
        ((false, false), Some("Save the file first")),
        ((true, false), None),
    ] {
        let ctx = egui::Context::default();
        // The first frame of a new area only sizes it.
        popup_frame(&ctx, state, (0.9, vec![]));
        let (_, painted) = popup_frame(&ctx, state, (1.0, vec![]));
        let (at, _) = painted
            .iter()
            .find(|(_, text)| text == "Stage hunk")
            .expect("the popup shows Stage hunk");
        let pos = *at + egui::vec2(4.0, 4.0);
        // Rest the pointer on the button: a disabled one says why.
        let mut texts = Vec::new();
        for frame in 0..12 {
            let events = if frame == 0 {
                vec![egui::Event::PointerMoved(pos)]
            } else {
                vec![]
            };
            let time = 1.1 + f64::from(frame) * 0.1;
            texts = popup_frame(&ctx, state, (time, events)).1;
        }
        if let Some(why) = why {
            assert!(texts.iter().any(|(_, text)| text == why), "{why}");
        }
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        popup_frame(&ctx, state, (2.5, vec![press(true)]));
        let (action, _) = popup_frame(&ctx, state, (2.6, vec![press(false)]));
        let expected = if why.is_none() {
            PopupAction::Stage
        } else {
            PopupAction::None
        };
        assert_eq!(action, expected, "{state:?}");
    }
}
