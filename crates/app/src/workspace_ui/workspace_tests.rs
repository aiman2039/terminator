use super::super::*;
use super::diff_paint::{DiffColors, paint_diff_document};
use super::diff_tests::tests::{long_doc, painted_text};
use super::header::{
    HEADER_ACTIONS, HOVER_POPUP_DELAY, ProjectHeaderChrome, filter_history_lines, header_row_width,
    header_visible_count, hover_popup_wake, project_header_chrome, should_replace_hover_popup,
};
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
    fn hover_popup_waits_before_the_first_open() {
        let mut hover = None;
        assert!(!should_replace_hover_popup(
            &mut hover,
            "a",
            None,
            HOVER_POPUP_DELAY
        ));
        assert_eq!(hover.as_ref().map(|(key, _)| key.as_str()), Some("a"));
        hover.as_mut().unwrap().1 -= HOVER_POPUP_DELAY;
        assert!(should_replace_hover_popup(
            &mut hover,
            "a",
            None,
            HOVER_POPUP_DELAY
        ));
    }

    #[test]
    fn hover_popup_wake_rests_once_the_popup_is_open() {
        let hover = Some(("a".to_owned(), Instant::now()));
        assert!(hover_popup_wake(&hover, "a", Some("a"), HOVER_POPUP_DELAY).is_none());
        let wake = hover_popup_wake(&hover, "a", None, HOVER_POPUP_DELAY)
            .expect("a fresh hover wakes once");
        assert!(wake > Duration::from_millis(50) && wake <= HOVER_POPUP_DELAY);
    }

    #[test]
    fn an_open_hover_popup_keeps_its_target_when_the_pointer_crosses_another_file() {
        let mut hover = Some(("a".into(), Instant::now()));
        assert!(!should_replace_hover_popup(
            &mut hover,
            "b",
            Some("a"),
            HOVER_POPUP_DELAY
        ));
        assert_eq!(hover.as_ref().map(|(key, _)| key.as_str()), Some("a"));
        assert!(!should_replace_hover_popup(
            &mut hover,
            "b",
            Some("b"),
            HOVER_POPUP_DELAY
        ));
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
        assert_eq!(header_visible_count(full - 1.0, count), count - 1);
        let three = header_row_width(3, true);
        assert_eq!(header_visible_count(three, count), 3);
        assert_eq!(header_visible_count(three - 1.0, count), 2);
        assert_eq!(header_visible_count(0.0, count), 0);
    }

    fn split_frame(
        ctx: &egui::Context,
        doc: &diff::DiffDocument,
        sync: &mut f32,
        events: Vec<egui::Event>,
    ) -> Vec<(egui::Pos2, String)> {
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
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
                paint_diff_document(ui, doc, true, colors, "sync-regression", &mut ratio, sync);
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
        split_frame(&ctx, &doc, &mut sync, vec![]);
        split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        for _ in 0..4 {
            split_frame(
                &ctx,
                &doc,
                &mut sync,
                vec![egui::Event::PointerMoved(over_left), wheel(-60.0)],
            );
        }
        assert!(
            sync > 0.0,
            "wheel over the left pane must move the shared offset: {sync}"
        );
        let painted = split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        assert_split_rows_aligned(&painted);
    }

    #[test]
    fn split_fractional_wheel_input_accumulates() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let mut sync = 0.0;
        let over_left = egui::pos2(150.0, 150.0);
        split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        for _ in 0..8 {
            split_frame(
                &ctx,
                &doc,
                &mut sync,
                vec![egui::Event::PointerMoved(over_left), wheel(-0.2)],
            );
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
}
