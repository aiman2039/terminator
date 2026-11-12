use super::super::*;
use super::diff_paint::{DiffColors, PaintDiffDocumentInput, paint_diff_document};
use super::diff_tests::long_doc;
use super::header::{
    HEADER_ACTIONS, ProjectHeaderChrome, filter_history_lines, header_row_width,
    header_visible_count, project_header_chrome,
};
use super::hover::{advance_menu_modifier, link_menu_modifier_pressed};
use crate::app::painted_text;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_filter_matches_case_insensitively() {
        let text = "cargo build ok\nFAILED to link\nwarning: unused\n";
        assert_eq!(filter_history_lines(text, "").len(), 3);
        assert_eq!(filter_history_lines(text, "failed"), vec!["FAILED to link"]);
        assert_eq!(filter_history_lines(text, "CARGO"), vec!["cargo build ok"]);
        assert!(filter_history_lines(text, "missing").is_empty());
    }

    #[test]
    fn menu_modifier_requires_a_fresh_press_and_not_plain_hover() {
        for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::MAC_CMD] {
            assert_eq!(advance_menu_modifier(false, &[]), (false, false));
            assert_eq!(
                advance_menu_modifier(false, &[egui::Event::ModifiersChanged(modifiers)]),
                (true, true)
            );
            assert_eq!(advance_menu_modifier(true, &[]), (true, false));
            assert_eq!(
                advance_menu_modifier(
                    true,
                    &[egui::Event::ModifiersChanged(
                        modifiers | egui::Modifiers::SHIFT
                    )]
                ),
                (true, false)
            );
            assert_eq!(
                advance_menu_modifier(
                    true,
                    &[egui::Event::ModifiersChanged(egui::Modifiers::NONE)]
                ),
                (false, false)
            );
        }
    }

    #[test]
    fn copy_chords_do_not_request_a_link_menu() {
        for chord in [
            egui::Event::Copy,
            egui::Event::Text("\u{3}".into()),
            egui::Event::Key {
                key: egui::Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            },
        ] {
            assert_eq!(
                advance_menu_modifier(
                    false,
                    &[egui::Event::ModifiersChanged(egui::Modifiers::CTRL), chord]
                ),
                (true, false)
            );
        }
        assert_eq!(
            advance_menu_modifier(true, &[egui::Event::WindowFocused(false)]),
            (false, false)
        );
    }

    #[test]
    fn all_panes_observe_one_modifier_press_in_the_same_frame() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL)],
                ..Default::default()
            },
            |ui| {
                assert!(link_menu_modifier_pressed(ui.ctx()));
                assert!(link_menu_modifier_pressed(ui.ctx()));
            },
        );
        output.textures_delta.clear();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!link_menu_modifier_pressed(ui.ctx()));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn wide_project_header_keeps_icons_and_the_natural_name() {
        assert_eq!(
            project_header_chrome(400.0, 52.0),
            ProjectHeaderChrome::Icons { name: 52.0 }
        );
    }

    #[test]
    fn medium_project_header_truncates_the_name_before_the_menu() {
        // Three icons are 88px. A 140px row leaves 50px for an 80px name.
        assert_eq!(
            project_header_chrome(140.0, 80.0),
            ProjectHeaderChrome::Icons { name: 50.0 }
        );
    }

    #[test]
    fn narrow_project_header_moves_player_and_agents_into_the_menu() {
        // 110px cannot keep a 36px name plus three icons (126px).
        // The menu and hide control still fit, and the name gets the leftover 50px.
        assert_eq!(
            project_header_chrome(110.0, 80.0),
            ProjectHeaderChrome::Menu {
                name: 50.0,
                hide: false
            }
        );
    }

    #[test]
    fn tighter_project_header_puts_hide_in_the_menu() {
        assert_eq!(
            project_header_chrome(50.0, 80.0),
            ProjectHeaderChrome::Menu {
                name: 20.0,
                hide: true
            }
        );
        assert_eq!(
            project_header_chrome(20.0, 80.0),
            ProjectHeaderChrome::Menu {
                name: 0.0,
                hide: true
            }
        );
    }

    #[test]
    fn header_overflow_count_hides_only_what_does_not_fit() {
        let count = HEADER_ACTIONS.len();
        let full = header_row_width(count, false);
        assert_eq!(header_visible_count(full, count), count);
        // The menu is one more header square: it takes the last action's
        // place, so a row one point short moves two actions into it.
        assert_eq!(header_visible_count(full - 1.0, count), count - 2);
        let three = header_row_width(3, true);
        assert_eq!(header_visible_count(three, 8), 3);
        assert_eq!(header_visible_count(three - 1.0, 8), 2);
        assert_eq!(header_visible_count(0.0, count), 0);
    }

    struct SplitFrameInput<'a> {
        ctx: &'a egui::Context,
        doc: &'a diff::DiffDocument,
        sync: &'a mut f32,
        events: Vec<egui::Event>,
    }

    fn split_frame(input: SplitFrameInput<'_>) -> Vec<(egui::Pos2, String)> {
        let SplitFrameInput {
            ctx,
            doc,
            sync,
            events,
        } = input;
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            moved: Color32::LIGHT_BLUE,
            text: Color32::WHITE,
        };
        let mut ratio = 0.5;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                paint_diff_document(PaintDiffDocumentInput {
                    ui,
                    doc,
                    split: true,
                    colors,
                    scroll_key: "sync-regression",
                    ratio: &mut ratio,
                    sync,
                });
            },
        );
        output.textures_delta.clear();
        painted_text(&output.shapes)
    }

    fn wheel(delta_y: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, delta_y),
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Every distinct painted text must appear at the same y in both panes, so
    /// corresponding split rows stay aligned.
    fn assert_split_rows_aligned(painted: &[(egui::Pos2, String)]) {
        use std::collections::HashMap;
        let mut ys: HashMap<&str, Vec<f32>> = HashMap::new();
        for (pos, text) in painted {
            ys.entry(text.as_str()).or_default().push(pos.y);
        }
        let mut compared: usize = 0;
        for (text, values) in &ys {
            if values.len() == 2
                && let (Some(left), Some(right)) = (values.first(), values.get(1))
            {
                compared = compared.saturating_add(1);
                assert!(
                    (*left - *right).abs() <= 1.0,
                    "row {text:?} is misaligned across panes: {values:?}"
                );
            }
        }
        assert!(compared > 0, "no corresponding rows were painted");
    }

    #[test]
    fn split_wheel_over_a_pane_moves_both_panes_in_step() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let mut sync = 0.0;
        let over_left = egui::pos2(150.0, 150.0);
        split_frame(SplitFrameInput {
            ctx: &ctx,
            doc: &doc,
            sync: &mut sync,
            events: vec![],
        });
        split_frame(SplitFrameInput {
            ctx: &ctx,
            doc: &doc,
            sync: &mut sync,
            events: vec![egui::Event::PointerMoved(over_left)],
        });
        for _ in 0..4 {
            split_frame(SplitFrameInput {
                ctx: &ctx,
                doc: &doc,
                sync: &mut sync,
                events: vec![egui::Event::PointerMoved(over_left), wheel(-60.0)],
            });
        }
        assert!(
            sync > 0.0,
            "wheel over the left pane must move the shared offset: {sync}"
        );
        let painted = split_frame(SplitFrameInput {
            ctx: &ctx,
            doc: &doc,
            sync: &mut sync,
            events: vec![egui::Event::PointerMoved(over_left)],
        });
        assert_split_rows_aligned(&painted);
    }

    #[test]
    fn split_fractional_wheel_input_accumulates() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let mut sync = 0.0;
        let over_left = egui::pos2(150.0, 150.0);
        split_frame(SplitFrameInput {
            ctx: &ctx,
            doc: &doc,
            sync: &mut sync,
            events: vec![egui::Event::PointerMoved(over_left)],
        });
        for _ in 0..8 {
            split_frame(SplitFrameInput {
                ctx: &ctx,
                doc: &doc,
                sync: &mut sync,
                events: vec![egui::Event::PointerMoved(over_left), wheel(-0.2)],
            });
        }
        assert!(
            sync > 0.0,
            "sub-half-point wheel input must accumulate instead of being dropped: {sync}"
        );
    }

    #[test]
    fn closing_a_diff_forgets_its_split_scroll() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            remote: None,
            cwd: dir.path().into(),
            path: dir.path().join("file.rs"),
            staged: false,
        };
        let key = tab.key();
        app.layouts.insert(
            "project".into(),
            Workspace::from_layout(DockState::new(vec![tab])),
        );
        app.diff_split_scroll.insert(key.clone(), 42.0);
        app.prune_diff_docs();
        assert!(
            app.diff_split_scroll.contains_key(&key),
            "an open diff keeps its cached scroll"
        );
        app.layouts.clear();
        app.prune_diff_docs();
        assert!(
            !app.diff_split_scroll.contains_key(&key),
            "closing the diff tab must forget its cached split scroll"
        );
    }

    /// One frame of the terminal find field and its keys. `focus` puts
    /// the keyboard in the field first. Returns whether the bar closes.
    fn find_frame(
        ctx: &egui::Context,
        find: &mut TerminalFind,
        focus: bool,
        key: Option<(egui::Key, egui::Modifiers)>,
    ) -> bool {
        let held = key.map_or(egui::Modifiers::NONE, |(_, modifiers)| modifiers);
        let mut events = vec![egui::Event::ModifiersChanged(held)];
        events.extend(key.map(|(key, modifiers)| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }));
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut close = false;
        let mut output = ctx.run_ui(raw, |ui| {
            let response = ui.add(appearance::singleline(&mut find.query));
            if focus {
                response.request_focus();
            }
            close = super::super::viewer::Viewer::find_bar_nav(ui, find, &response).1;
        });
        output.textures_delta.clear();
        close
    }

    #[test]
    fn enter_shift_enter_and_esc_work_in_the_terminal_find_bar() {
        let ctx = egui::Context::default();
        let hit = |line| egui_term::FoundMatch {
            line,
            start_col: 0,
            end_col: 1,
        };
        let mut find = TerminalFind {
            query: "x".into(),
            ..Default::default()
        };
        find.outcome.matches = vec![hit(0), hit(1), hit(2)];
        let press = |key| Some((key, egui::Modifiers::NONE));
        find_frame(&ctx, &mut find, true, None);
        find_frame(&ctx, &mut find, false, press(egui::Key::Enter));
        assert_eq!(find.current, 1, "Enter moves to the next match");
        find_frame(&ctx, &mut find, false, press(egui::Key::Enter));
        assert_eq!(find.current, 2, "the field keeps focus after Enter");
        let shift_enter = Some((egui::Key::Enter, egui::Modifiers::SHIFT));
        find_frame(&ctx, &mut find, false, shift_enter);
        assert_eq!(find.current, 1, "Shift+Enter moves back");
        assert!(find_frame(&ctx, &mut find, false, press(egui::Key::Escape)));
    }
}
