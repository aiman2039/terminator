use super::*;

#[test]
fn change_bands_distinguish_replacements_additions_and_deletions() {
    let colors = super::super::diff_paint::DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
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
        if hunk.replacement() {
            assert_eq!(row.added, Color32::BLUE);
            assert_eq!(row.deleted, Color32::BLUE);
        }
    }
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
                        aligned: false,
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
