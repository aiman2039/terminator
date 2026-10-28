use super::*;

#[test]
fn highlighted_editor_job_keeps_buffer_text_and_tints() {
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "let a = 1;\n",
        right: "let a = 2;\n",
        staged: false,
        context: 3,
    });
    let colors = DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        text: Color32::WHITE,
    };
    let right_tints = super::super::diff_edit::live_tints(&doc.left_text, &doc.right_text, colors);
    let spans = diff::highlight_spans(std::path::Path::new("file.rs"), &doc.right_text);
    let valid: Vec<bool> = spans.iter().map(|line| !line.is_empty()).collect();
    let job =
        super::super::diff_edit::highlighted_job(super::super::diff_edit::HighlightedJobInput {
            text: &doc.right_text,
            spans: &spans,
            valid: &valid,
            tints: &right_tints,
            font: egui::FontId::monospace(12.0),
            fallback: Color32::WHITE,
            wrap_width: f32::INFINITY,
        });
    // A TextEdit layouter must reproduce the buffer text exactly.
    assert_eq!(job.text, doc.right_text);
    // The changed line carries a diff tint background.
    assert!(right_tints.iter().any(|tint| tint.is_some()));
}

#[test]
fn live_tints_track_edits_as_they_happen() {
    let colors = DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        text: Color32::WHITE,
    };
    let tints = super::super::diff_edit::live_tints("a\nb\nc\n", "a\nB\nc\nd\n", colors);
    assert_eq!(tints[0], None);
    assert!(tints[1].is_some(), "replaced line is tinted");
    assert_eq!(tints[2], None);
    assert!(tints[3].is_some(), "inserted line is tinted");
    // Fixing the change removes its tint immediately.
    let clean = super::super::diff_edit::live_tints("a\nb\n", "a\nb\n", colors);
    assert!(clean.iter().all(|tint| tint.is_none()));
    // An insertion shifts later tints with the text.
    let shifted = super::super::diff_edit::live_tints("a\nb\n", "x\na\nb\n", colors);
    assert!(shifted[0].is_some());
    assert_eq!(shifted[1], None);
}

#[cfg(feature = "test-support")]
#[test]
fn typing_in_unified_diff_edits_the_buffer_and_tints_live() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "let a = 1;\nlet b = 2;\n",
        right: "let a = 1;\nlet b = 3;\n",
        staged: false,
        context: 3,
    });
    app.diffs.insert(tab.key(), Ok(doc.into()));
    // Edit the already-changed line 2.
    click_and_type(ClickAndTypeInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-2",
        text: "X",
    });
    let text = buffer_text(&app, &tab);
    assert!(
        text.contains('X'),
        "unified typing must reach the buffer, got {text:?}"
    );
    // Changing the untouched line 1 must tint it in the same session.
    click_and_type(ClickAndTypeInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-1",
        text: "Y",
    });
    assert!(
        buffer_text(&app, &tab).contains('Y'),
        "line 1 typing must reach the buffer, got {:?}",
        buffer_text(&app, &tab)
    );
    let review = app.diff_reviews.get(&tab.key()).expect("review state");
    let buffer = review.buffer.as_ref().expect("buffer");
    let (_, tints) = buffer.live_tints.as_ref().expect("live tints cached");
    assert!(
        tints.first().is_some_and(|tint| tint.is_some()),
        "line 1 must tint live after editing it, tints: {tints:?}"
    );
    assert!(buffer_text(&app, &tab).contains('Y'));
}

#[cfg(feature = "test-support")]
#[test]
fn typing_in_split_diff_edits_the_buffer_and_tints_live() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    app.diff_split.insert(tab.key());
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "let a = 1;\nlet b = 2;\n",
        right: "let a = 1;\nlet b = 3;\n",
        staged: false,
        context: 3,
    });
    app.diffs.insert(tab.key(), Ok(doc.into()));
    click_and_type(ClickAndTypeInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-right-editor",
        text: "Z",
    });
    let text = buffer_text(&app, &tab);
    assert!(
        text.contains('Z'),
        "split typing must reach the buffer, got {text:?}"
    );
    let review = app.diff_reviews.get(&tab.key()).expect("review state");
    let buffer = review.buffer.as_ref().expect("buffer");
    // Tint and syntax caches must describe the text just typed, not the
    // document the tab was opened with.
    let tint_key = buffer.live_tints.as_ref().expect("live tints cached").0;
    let span_key = buffer.right_spans.as_ref().expect("spans cached").0;
    assert_eq!(tint_key, span_key, "caches must track the edited revision");
    assert!(
        buffer
            .live_tints
            .as_ref()
            .is_some_and(|(_, tints)| tints.iter().any(|tint| tint.is_some())),
        "edited lines must carry a live tint"
    );
}

#[test]
fn live_rows_cover_the_whole_file_and_track_blocks() {
    use diff::review::{LiveRow, LiveTint};
    let rows = super::super::diff_edit::live_rows("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
    // Delete of b (Gone) + B (Changed) + appended e (Added), rest equal.
    assert!(matches!(rows[0], LiveRow::Edit { changed: None, .. }));
    assert!(matches!(rows[1], LiveRow::Gone { old: 1, .. }));
    assert!(matches!(
        rows[2],
        LiveRow::Edit {
            changed: Some(LiveTint::Changed),
            ..
        }
    ));
    assert!(matches!(
        rows[5],
        LiveRow::Edit {
            changed: Some(LiveTint::Added),
            ..
        }
    ));
    let edits = rows
        .iter()
        .filter(|row| matches!(row, LiveRow::Edit { .. }))
        .count();
    assert_eq!(edits, 5, "every buffer line is an editable row");
    assert_eq!(
        super::super::diff_edit::replace_line("", 0, "x"),
        "x",
        "an empty file gains its first line"
    );
}

#[test]
fn row_ops_split_and_join_lines() {
    use super::super::diff_edit::{RowOp, apply_row_op};
    let split = apply_row_op(
        "alpha\nbeta\n",
        &RowOp::Split {
            line: 0,
            head: "al".into(),
            tail: "pha".into(),
        },
    );
    assert_eq!(split, "al\npha\nbeta\n");
    let joined = apply_row_op(&split, &RowOp::JoinPrev { line: 1 });
    assert_eq!(joined, "alpha\nbeta\n");
    let joined_next = apply_row_op("alpha\nbeta\n", &RowOp::JoinNext { line: 0 });
    assert_eq!(joined_next, "alphabeta\n");
    // Out-of-range ops are no-ops.
    assert_eq!(apply_row_op("a\n", &RowOp::JoinPrev { line: 0 }), "a\n");
}

#[cfg(feature = "test-support")]
#[test]
fn undo_redo_in_unified_restores_buffer_text() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    let original = "let a = 1;\nlet b = 3;\n";
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "let a = 1;\nlet b = 2;\n",
        right: original,
        staged: false,
        context: 3,
    });
    app.diffs.insert(tab.key(), Ok(doc.into()));
    click_and_type(ClickAndTypeInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-1",
        text: "X",
    });
    let edited = buffer_text(&app, &tab);
    assert_ne!(edited, original, "typing must change the buffer first");
    click_and_shortcut(ClickAndShortcutInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-1",
        shift: false,
    });
    assert_eq!(
        buffer_text(&app, &tab),
        original,
        "Cmd+Z must restore the pre-edit buffer"
    );
    click_and_shortcut(ClickAndShortcutInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-1",
        shift: true,
    });
    assert_eq!(
        buffer_text(&app, &tab),
        edited,
        "Cmd+Shift+Z must redo the edit"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn enter_in_unified_inserts_a_new_line() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "let a = 1;\nlet b = 2;\n",
        right: "let a = 1;\nlet b = 3;\n",
        staged: false,
        context: 3,
    });
    app.diffs.insert(tab.key(), Ok(doc.into()));
    click_and_key(ClickAndKeyInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-2",
        key: egui::Key::Enter,
    });
    let text = buffer_text(&app, &tab);
    assert!(
        text.contains(";\n") && text.lines().count() >= 3,
        "Enter must split the unified row into a new line, got {text:?}"
    );
    // Focus must follow the new line so typing continues there.
    let focused = ctx.memory(|memory| memory.focused());
    assert_eq!(
        focused,
        Some(egui::Id::unique((tab.key(), "unified-line", 3_u32))),
        "focus must move to the split line"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn typing_in_unified_gap_line_edits_and_highlights_its_block() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    let left: String = (1..=12).fold(String::new(), |mut acc, n| {
        use std::fmt::Write as _;
        let _ = writeln!(acc, "line {n}");
        acc
    });
    let mut right = left.clone();
    right = right.replace("line 12", "line twelve");
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: &left,
        right: &right,
        staged: false,
        context: 3,
    });
    app.diffs.insert(tab.key(), Ok(doc.into()));
    // Line 1 sits in a collapsed gap in the old grouped view; it must be
    // editable here exactly like in split view.
    click_and_type(ClickAndTypeInput {
        app: &mut app,
        ctx: &ctx,
        tab: &tab,
        target: "diff-unified-edit-1",
        text: "Q",
    });
    assert!(
        buffer_text(&app, &tab).lines().next() == Some("line 1Q"),
        "gap line must accept edits, got {:?}",
        buffer_text(&app, &tab)
    );
    let review = app.diff_reviews.get(&tab.key()).expect("review state");
    let buffer = review.buffer.as_ref().expect("buffer");
    let (_, rows) = buffer.live_rows.as_ref().expect("live rows cached");
    assert!(
        rows.iter().any(|row| matches!(
            row,
            diff::review::LiveRow::Edit {
                new: 0,
                changed: Some(_),
                ..
            }
        )),
        "the edited gap line must join a highlighted block live: {rows:?}"
    );
}
