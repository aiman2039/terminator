use super::*;

#[test]
fn change_bands_distinguish_replacements_additions_and_deletions() {
    let colors = super::super::diff_paint::DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        moved: Color32::LIGHT_BLUE,
        text: Color32::WHITE,
    };
    for (old, new, expected) in [
        ("old\n", "new\n", Color32::BLUE),
        ("", "new\n", Color32::GREEN),
        ("old\n", "", Color32::RED),
    ] {
        let doc = diff::build_with_context(diff::BuildWithContextInput {
            path: std::path::Path::new("file.txt"),
            left: old,
            right: new,
            staged: false,
            context: 3,
        });
        let hunk = doc.hunks.first().unwrap();
        assert_eq!(change_color(hunk, colors), expected);
        let row = super::super::diff_paint::row_colors(super::super::diff_paint::RowColorsInput {
            doc: &doc,
            row: hunk.row(true),
            split: true,
            colors,
        });
        // Rows keep the side colors, a replacement included; only the
        // band and rail use the modified color.
        assert_eq!(row.added, Color32::GREEN);
        assert_eq!(row.deleted, Color32::RED);
    }
}

#[test]
fn removed_and_added_rows_use_different_tints() {
    use super::super::diff_paint::{DiffColors, RowColorsInput, diff_row_style, row_colors};
    let colors = DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        moved: Color32::LIGHT_BLUE,
        text: Color32::WHITE,
    };
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.txt"),
        left: "a\nold\nc\n",
        right: "a\nnew\nc\n",
        staged: false,
        context: 3,
    });
    let hunk = doc.hunks.first().unwrap();
    assert!(hunk.replacement());
    let row = row_colors(RowColorsInput {
        doc: &doc,
        row: hunk.row(false),
        split: false,
        colors,
    });
    let (removed, minus) = diff_row_style(diff::LineKind::Delete, row);
    let (added, plus) = diff_row_style(diff::LineKind::Insert, row);
    assert_ne!(removed, added);
    assert_eq!((minus, plus), ("-", "+"));
    let alpha = colors.tints().row;
    assert_eq!(
        removed,
        Color32::from_rgba_unmultiplied(255, 0, 0, alpha),
        "removed rows are red"
    );
    assert_eq!(
        added,
        Color32::from_rgba_unmultiplied(0, 255, 0, alpha),
        "added rows are green"
    );
}

#[test]
fn changed_words_use_a_stronger_tint_of_the_row_hue() {
    use super::super::diff_paint::{DiffColors, diff_code_job, diff_row_style};
    let colors = DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        moved: Color32::LIGHT_BLUE,
        text: Color32::WHITE,
    };
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.txt"),
        left: "let a = 1;\n",
        right: "let a = 2;\n",
        staged: false,
        context: 3,
    });
    let font = egui::FontId::monospace(appearance::FONT_SMALL);
    for kind in [diff::LineKind::Delete, diff::LineKind::Insert] {
        let line = doc.unified.iter().find(|line| line.kind == kind).unwrap();
        let (row, _) = diff_row_style(kind, colors);
        let job = diff_code_job(line, &font, colors);
        let word = job
            .sections
            .iter()
            .map(|section| section.format.background)
            .find(|bg| *bg != Color32::TRANSPARENT)
            .expect("a changed word is highlighted");
        let unmultiplied = |c: Color32| c.to_srgba_unmultiplied();
        let ([wr, wg, wb, wa], [rr, rg, rb, ra]) = (unmultiplied(word), unmultiplied(row));
        assert_eq!((wr, wg, wb), (rr, rg, rb), "same hue as the row: {kind:?}");
        assert!(wa > ra, "stronger than the row: {kind:?}");
    }
}

#[test]
fn every_theme_tints_words_stronger_than_rows() {
    for dark in [true, false] {
        let tints = appearance::diff_tints(dark);
        assert!(tints.row > 0, "rows stay visible; dark={dark}");
        assert!(tints.word > tints.row, "dark={dark}");
    }
}

#[test]
fn the_focused_hunk_is_a_thin_bar_on_its_left_edge() {
    let band = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 48.0));
    let bar = focus_bar(band);
    assert_eq!(bar.left(), band.left());
    assert_eq!(bar.y_range(), band.y_range());
    assert_eq!(bar.width(), FOCUS_BAR_WIDTH);
    assert_eq!(FOCUS_BAR_WIDTH, 2.0);
    let colors = super::super::diff_paint::DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::YELLOW,
        moved: Color32::LIGHT_BLUE,
        text: Color32::WHITE,
    };
    assert_eq!(colors.focus(), Color32::LIGHT_BLUE, "the theme accent");
}

#[test]
fn next_change_scrolls_to_the_hunk_in_both_views() {
    use std::fmt::Write;
    let mut original = String::new();
    for line in 0..1200 {
        writeln!(original, "line {line}").unwrap();
    }
    let changed = original
        .replace("line 20\n", "first change\n")
        .replace("line 1100\n", "last change\n");
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.txt"),
        left: &original,
        right: &changed,
        staged: false,
        context: 2000,
    });
    for split in [false, true] {
        let ctx = egui::Context::default();
        let mut review = diff::review::State::default();
        review.select(1);
        let mut sync = 0.0;
        let mut ratio = 0.5;
        let mut visible = false;
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let colors = super::super::diff_paint::DiffColors {
                        added: Color32::GREEN,
                        deleted: Color32::RED,
                        accent: Color32::BLUE,
                        moved: Color32::LIGHT_BLUE,
                        text: Color32::WHITE,
                    };
                    let scroll = super::super::diff_paint::paint_diff_document(
                        super::super::diff_paint::PaintDiffDocumentInput {
                            ui,
                            doc: &doc,
                            split,
                            colors,
                            scroll_key: "jump-regression",
                            ratio: &mut ratio,
                            sync: &mut sync,
                        },
                    );
                    for output in &scroll {
                        let top = output.inner_rect.top()
                            + doc.hunks.get(1).unwrap().row(split) as f32 * diff_row_height(ui)
                            - output.state.unclamped_offset().y;
                        visible = output.state.unclamped_offset().y > 1000.0
                            && output.inner_rect.y_range().contains(top);
                        assert!(
                            !visible || top + diff_row_height(ui) <= output.inner_rect.bottom()
                        );
                    }
                    jump(JumpInput {
                        ui,
                        doc: &doc,
                        split,
                        row_h: None,
                        review: &mut review,
                        scroll: &scroll,
                        sync: &mut sync,
                    });
                },
            );
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "the selected hunk must enter the viewport; split={split}"
        );
    }
}

#[test]
fn chevrons_route_to_discard_stage_and_unstage() {
    use super::chevrons::{Chevron, chevron_actions};
    assert_eq!(
        chevron_actions(false),
        [
            (Chevron::Revert, Kind::Discard),
            (Chevron::Apply, Kind::Stage)
        ]
    );
    assert_eq!(chevron_actions(true), [(Chevron::Revert, Kind::Unstage)]);
}

/// Click the `>>` chevron of the only hunk in a split view and return the
/// action the controls asked for. `stage_lock` is a running commit.
fn click_revert_chevron(staged: bool, stage_lock: bool) -> Option<(usize, Kind)> {
    let doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.txt"),
        left: "a\nb\nc\n",
        right: "a\nB\nc\n",
        staged,
        context: 3,
    });
    let colors = super::super::diff_paint::DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        moved: Color32::LIGHT_BLUE,
        text: Color32::WHITE,
    };
    let ctx = egui::Context::default();
    let mut review = diff::review::State::default();
    let (mut sync, mut ratio) = (0.0, 0.5);
    let mut target = egui::Pos2::ZERO;
    let mut action = None;
    for frame in 0..4 {
        let events = match frame {
            1 => vec![egui::Event::PointerMoved(target)],
            2 | 3 => vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: frame == 2,
                modifiers: egui::Modifiers::default(),
            }],
            _ => Vec::new(),
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let scroll = super::super::diff_paint::paint_diff_document(
                    super::super::diff_paint::PaintDiffDocumentInput {
                        ui,
                        doc: &doc,
                        split: true,
                        colors,
                        scroll_key: "chevron-click",
                        ratio: &mut ratio,
                        sync: &mut sync,
                    },
                );
                let row_h = diff_row_height(ui);
                let hunk = doc.hunks.first().unwrap();
                let first = scroll.first().unwrap();
                // No hover first: the chevron is already there to click.
                target = egui::pos2(
                    first.inner_rect.right() + 7.0,
                    first.inner_rect.top() + hunk.row(true) as f32 * row_h + row_h.max(12.0) / 2.0,
                );
                let interaction = paint(PaintInput {
                    ui,
                    doc: &doc,
                    split: true,
                    staged,
                    editor_pane: false,
                    enabled: true,
                    stage_lock,
                    review: &mut review,
                    scroll: &scroll,
                    rail: egui::Rect::from_min_size(egui::pos2(792.0, 0.0), egui::vec2(8.0, 400.0)),
                    unified_gutter: egui::Rect::ZERO,
                    colors,
                    row_h: None,
                });
                action = action.or(interaction.action);
            },
        );
        output.textures_delta.clear();
    }
    action
}

#[test]
fn split_chevron_click_discards_working_changes_and_unstages_staged_ones() {
    assert_eq!(click_revert_chevron(false, false), Some((0, Kind::Discard)));
    assert_eq!(click_revert_chevron(true, false), Some((0, Kind::Unstage)));
}

#[test]
fn a_running_commit_disables_the_unstage_chevron_but_not_discard() {
    assert_eq!(click_revert_chevron(true, true), None);
    assert_eq!(click_revert_chevron(false, true), Some((0, Kind::Discard)));
}

#[test]
fn gutter_glyphs_have_one_verb_each_and_unstage_is_not_an_arrow() {
    use super::chevrons::action_icon;
    assert_eq!(action_icon(Kind::Stage), "Plus");
    assert_eq!(action_icon(Kind::Unstage), "Minus");
    assert_eq!(action_icon(Kind::Discard), "Eraser");
    for kind in [Kind::Stage, Kind::Unstage, Kind::Discard] {
        assert!(crate::icons::is_ui_icon(action_icon(kind)));
    }
}

#[test]
fn hunk_tips_show_the_diff_key_only_on_the_selected_change() {
    use super::chevrons::action_tip;
    assert!(action_tip(Kind::Stage, 0, true).ends_with("(s)"));
    assert!(action_tip(Kind::Unstage, 0, true).ends_with("(u)"));
    assert!(action_tip(Kind::Discard, 2, true).ends_with("(x)"));
    for kind in [Kind::Stage, Kind::Unstage, Kind::Discard] {
        let (plain, keyed) = (action_tip(kind, 0, false), action_tip(kind, 0, true));
        assert_eq!(keyed.strip_prefix(&plain).map(str::len), Some(4));
    }
}
