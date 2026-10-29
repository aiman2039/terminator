use super::*;

fn working_tab() -> Tab {
    Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    }
}

fn editable_app(ctx: &egui::Context, dir: &std::path::Path, (left, right): (&str, &str)) -> App {
    crate::appearance::install(ctx);
    let mut app = App::with_context(ctx, Paths::at(dir.into()));
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left,
        right,
        staged: false,
        context: 3,
    });
    app.diffs.insert(working_tab().key(), Ok(doc.into()));
    app
}

#[test]
fn split_editor_rows_line_up_with_the_left_pane_across_deletions() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = editable_app(&ctx, dir.path(), ("keep\ngone\ntail\n", "keep\ntail\n"));
    let tab = working_tab();
    app.diff_split.insert(tab.key());
    let painted = paint_diff_view(&mut app, &ctx, &tab);
    let rows: Vec<f32> = painted
        .iter()
        .filter(|(_, text)| text.trim() == "tail")
        .map(|(pos, _)| pos.y)
        .collect();
    assert_eq!(rows.len(), 2, "both panes paint `tail`: {painted:?}");
    assert!(
        (rows[0] - rows[1]).abs() < 1.0,
        "a filler row keeps `tail` on one row in both panes: {rows:?}"
    );
    let gone = require_text(&painted, "gone").0;
    assert!(gone.y < rows[0], "the deleted line keeps its own row");
}

#[cfg(feature = "test-support")]
#[test]
fn typing_in_split_diff_edits_the_buffer_and_re_diffs_live() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = editable_app(
        &ctx,
        dir.path(),
        ("let a = 1;\nlet b = 2;\n", "let a = 1;\nlet b = 3;\n"),
    );
    app.state.settings.native_vim = false;
    let tab = working_tab();
    app.diff_split.insert(tab.key());
    click_and_key(ClickAndKeyInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-right-editor",
        key: egui::Key::Z,
    });
    let text = buffer_text(&app, &tab);
    assert!(
        text.contains('z'),
        "split typing must reach the buffer: {text:?}"
    );
    let review = app.diff_reviews.get(&tab.key()).expect("review state");
    assert!(review.unsaved_edits());
    let buffer = review.buffer.as_ref().expect("buffer");
    let (revision, live) = buffer.live.shown.as_ref().expect("live diff");
    assert_eq!(
        *revision,
        terminator_native_edit::doc::Buffer::revision(&buffer.editor.doc),
        "the live diff describes the text just typed"
    );
    assert_eq!(live.right_text, text);
}

#[cfg(feature = "test-support")]
#[test]
fn vim_keys_edit_the_split_diff_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = editable_app(&ctx, dir.path(), ("a\n", "b\n"));
    app.state.settings.native_vim = true;
    let tab = working_tab();
    app.diff_split.insert(tab.key());
    click_and_key(ClickAndKeyInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-right-editor",
        key: egui::Key::I,
    });
    let base = ctx.input(|input| input.time);
    run_diff_frames(RunDiffFramesInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        frames: &[
            (base + 0.1, vec![egui::Event::Text("Q".into())]),
            (base + 0.2, vec![]),
        ],
    });
    assert!(
        buffer_text(&app, &tab).contains('Q'),
        "`i` enters Insert mode, then `Q` is typed: {:?}",
        buffer_text(&app, &tab)
    );
}

#[cfg(feature = "test-support")]
#[test]
fn undo_in_split_editor_restores_the_saved_text() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let original = "let a = 1;\nlet b = 3;\n";
    let mut app = editable_app(&ctx, dir.path(), ("let a = 1;\nlet b = 2;\n", original));
    app.state.settings.native_vim = false;
    let tab = working_tab();
    app.diff_split.insert(tab.key());
    click_and_key(ClickAndKeyInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-right-editor",
        key: egui::Key::X,
    });
    assert_ne!(buffer_text(&app, &tab), original);
    click_and_shortcut(ClickAndShortcutInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-right-editor",
        shift: false,
    });
    assert_eq!(buffer_text(&app, &tab), original, "Cmd+Z undoes the edit");
    let review = app.diff_reviews.get(&tab.key()).expect("review state");
    assert!(!review.unsaved_edits(), "undone edits are not unsaved");
}

#[cfg(feature = "test-support")]
#[test]
fn unified_working_tree_is_read_only_with_an_edit_switch_to_split() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = editable_app(&ctx, dir.path(), ("a\n", "b\n"));
    let tab = working_tab();
    let painted = paint_diff_view(&mut app, &ctx, &tab);
    require_text(&painted, "b");
    assert!(
        fixture_target(&ctx, "diff-right-editor").is_none(),
        "unified mode shows no editor"
    );
    let edit = fixture_target(&ctx, "diff-edit-split").expect("Edit button");
    let pos = edit.center();
    run_diff_frames(RunDiffFramesInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        frames: &[
            (1.0, vec![egui::Event::PointerMoved(pos)]),
            (
                1.1,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            ),
            (
                1.2,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ),
            (1.3, vec![]),
        ],
    });
    assert!(
        app.diff_split.contains(&tab.key()),
        "Edit opens side by side"
    );
    assert!(fixture_target(&ctx, "diff-right-editor").is_some());
}
