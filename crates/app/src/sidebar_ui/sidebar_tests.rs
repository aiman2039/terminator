use super::attention::*;
use super::explorer_rows::*;
use super::git_panel::*;
use crate::agent_presence::attention_status_icon;
use crate::{
    appearance, file_actions::FileAction, icons, preferences::ExplorerSearchMode,
    services::ContextData, workspace_ops,
};
use eframe::egui::{self};
use std::path::{Path, PathBuf};
use terminator_core::appearance::AppearanceConfig;
use terminator_core::{AgentState, Notification, ReviewMode};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_badge_label_covers_the_count_matrix() {
        assert_eq!(attention_badge_label(0, 0), "");
        assert_eq!(attention_badge_label(0, 2), "2 unread");
        assert_eq!(attention_badge_label(3, 0), "3 waiting");
        assert_eq!(attention_badge_label(3, 2), "3 waiting · 2 unread");
    }

    #[test]
    fn explorer_query_filter_is_case_insensitive_and_keeps_directories() {
        // The needle is the pre-lowercased query hoisted out of the per-file
        // loop; labels may use any case.
        assert!(explorer_query_keeps_file("README.md", false, None));
        assert!(explorer_query_keeps_file("README.md", false, Some("read")));
        assert!(explorer_query_keeps_file("MAIN.RS", false, Some("main")));
        assert!(!explorer_query_keeps_file("main.rs", false, Some("test")));
        assert!(explorer_query_keeps_file("src", true, Some("test")));
        assert!(explorer_query_keeps_file("src", true, None));
    }

    #[test]
    fn inbox_rows_fold_notices_per_session() {
        fn notice(id: &str, session: &str, invocation: &str, created: u64) -> Notification {
            Notification {
                id: id.into(),
                session_id: session.into(),
                invocation_id: invocation.into(),
                request_id: None,
                state: AgentState::WaitingInput,
                summary: String::new(),
                details: String::new(),
                created,
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            }
        }
        let groups = group_notices(vec![
            notice("n1", "s", "a", 3),
            notice("n2", "s", "a", 2),
            notice("n3", "s", "b", 1),
            notice("n4", "t", "a", 0),
        ]);
        // Repeated agent runs in one session fold into a single row.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].notices.len(), 3);
        assert_eq!(groups[0].notices[0].id, "n1");
        assert_eq!(groups[0].notices[1].id, "n2");
        assert_eq!(groups[0].notices[2].id, "n3");
        assert_eq!(groups[1].notices[0].id, "n4");
        assert!(group_notices(vec![]).is_empty());
    }

    #[test]
    fn git_click_actions_route_by_viewer_preference_and_capability() {
        use FileAction::*;
        assert_eq!(ReviewMode::default(), ReviewMode::Native);
        for (deleted, staged, clicked, mode, advertised, expected) in [
            (false, None, true, ReviewMode::Native, false, Some(Open)),
            (false, None, true, ReviewMode::Neovim, true, Some(Open)),
            (true, None, true, ReviewMode::Native, false, None),
            (
                false,
                Some(false),
                true,
                ReviewMode::Native,
                false,
                Some(NativeWorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Native,
                true,
                Some(NativeStagedDiff),
            ),
            (
                false,
                Some(false),
                true,
                ReviewMode::Neovim,
                false,
                Some(NativeWorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Neovim,
                false,
                Some(NativeStagedDiff),
            ),
            (
                false,
                Some(false),
                true,
                ReviewMode::Neovim,
                true,
                Some(WorkingDiff),
            ),
            (
                false,
                Some(true),
                true,
                ReviewMode::Neovim,
                true,
                Some(StagedDiff),
            ),
            (
                true,
                Some(false),
                true,
                ReviewMode::Neovim,
                true,
                Some(WorkingDiff),
            ),
            (false, Some(false), false, ReviewMode::Neovim, true, None),
        ] {
            assert_eq!(
                git_click_action(deleted, staged, clicked, mode, advertised),
                expected,
                "deleted={deleted} staged={staged:?} clicked={clicked} mode={mode:?} advertised={advertised}"
            );
        }
    }

    #[test]
    fn clipped_attention_cards_keep_the_same_height_as_visible_cards() {
        let notice = Notification {
            id: "notice".into(),
            session_id: "session".into(),
            invocation_id: "agent".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: "waiting".into(),
            details: String::new(),
            created: 1,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        };
        let theme = AppearanceConfig::default();
        for selected in [false, true] {
            let measure = |clipped: bool| {
                let ctx = egui::Context::default();
                appearance::install(&ctx);
                let mut height = 0.0;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    if clipped {
                        ui.set_clip_rect(egui::Rect::NOTHING);
                    }
                    let start = ui.next_widget_position().y;
                    attention_card(
                        ui,
                        AttentionCard {
                            theme: &theme,
                            notice: &notice,
                            session: None,
                            selected,
                            highlight: false,
                            brand_icon: None,
                            brand_label: None,
                            show_read: true,
                            group_extra: &[],
                        },
                    );
                    height = ui.next_widget_position().y - start;
                });
                output.textures_delta.clear();
                if clipped {
                    assert!(
                        output
                            .shapes
                            .iter()
                            .all(|s| !matches!(s.shape, egui::Shape::Text(_)))
                    );
                }
                height
            };
            assert_eq!(measure(false), measure(true));
        }
    }

    #[test]
    fn variable_cards_reuse_measured_height_but_remeasure_after_resize() {
        let ctx = egui::Context::default();
        let mut draws = 0;
        let mut heights = Vec::new();
        for (width, clipped) in [(300.0, false), (300.0, true), (150.0, true)] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                if clipped {
                    ui.set_clip_rect(egui::Rect::NOTHING);
                }
                let start = ui.next_widget_position().y;
                cached_variable_card(ui, 42, |ui| {
                    draws += 1;
                    ui.group(|ui| {
                        ui.label(
                            "A long terminal message that wraps to multiple lines when resized",
                        );
                    })
                    .response
                    .rect
                });
                heights.push(ui.next_widget_position().y - start);
            });
            output.textures_delta.clear();
        }
        assert_eq!(draws, 2);
        assert_eq!(heights[0], heights[1]);
        assert!(heights[2] > heights[1]);
    }

    #[test]
    fn large_git_sidebar_only_builds_visible_rows() {
        let ctx = egui::Context::default();
        let mut rendered = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_clip_rect(egui::Rect::from_min_size(
                    ui.next_widget_position(),
                    egui::vec2(300.0, 400.0),
                ));
                let start = ui.next_widget_position().y;
                let stride = GIT_TREE_ROW_HEIGHT + ui.spacing().item_spacing.y;
                for _ in 0..23_315 {
                    if skip_clipped_git_row(ui) {
                        continue;
                    }
                    rendered += 1;
                    appearance::file_row(
                        ui,
                        "file.rs",
                        icons::file_icon(std::path::Path::new("file.rs")),
                        false,
                        GIT_TREE_ROW_HEIGHT,
                        "M",
                        egui::Color32::WHITE,
                    );
                }
                assert!((ui.next_widget_position().y - start - stride * 23_315.0).abs() < 2.0);
            });
        });
        output.textures_delta.clear();
        assert!(rendered > 0 && rendered < 40, "built {rendered} rows");
    }

    #[test]
    fn message_preview_preserves_words_and_punctuation_without_markdown() {
        assert_eq!(
            notice_preview("**No.** A different user or a `mini-VM` does not help."),
            "No. A different user or a mini-VM does not help."
        );
        assert_eq!(
            notice_preview("**Ready**.\n\n[Open](https://example.com)"),
            "Ready. Open"
        );
    }

    #[test]
    fn resolved_permission_requests_do_not_count_as_waiting() {
        let mut notice = Notification {
            id: "notice".into(),
            session_id: "session".into(),
            invocation_id: "agent".into(),
            request_id: None,
            state: AgentState::WaitingPermission,
            summary: String::new(),
            details: String::new(),
            created: 0,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        };
        assert!(notice_waiting(&notice));
        assert!(notice_pending(&notice, 10));
        notice.snoozed_until = 11;
        assert!(!notice_pending(&notice, 10));
        notice.snoozed_until = 0;
        notice.dismissed = true;
        assert!(!notice_pending(&notice, 10));
        notice.dismissed = false;
        notice.resolved = true;
        assert!(!notice_pending(&notice, 10));
        assert!(!notice_waiting(&notice));
        notice.state = AgentState::WaitingInput;
        assert!(!notice_waiting(&notice));
        notice.resolved = false;
        assert!(notice_waiting(&notice));
    }

    #[test]
    fn attention_status_icon_is_unique_per_state() {
        let states = [
            AgentState::Unknown,
            AgentState::Running,
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Completed,
            AgentState::Failed,
            AgentState::Stopped,
        ];
        let icons: Vec<_> = states.into_iter().map(attention_status_icon).collect();
        let unique: std::collections::HashSet<_> = icons.iter().copied().collect();
        assert_eq!(unique.len(), icons.len());
        for state in states {
            assert!(!attention_status_detail(state).is_empty());
            assert!(!state.label().is_empty());
        }
    }

    #[test]
    fn attention_label_width_leaves_the_action_row_intact() {
        let spacing = 4.0;
        let remaining = 200.0;
        let width = attention_label_width(remaining, spacing, false);
        let used = width + spacing + ATTENTION_ACTION_SIZE * ATTENTION_ACTION_COUNT;
        assert!((used - remaining).abs() < f32::EPSILON);
        assert_eq!(attention_label_width(10.0, spacing, false), 0.0);
        let read = attention_label_width(remaining, spacing, true);
        assert!(read < width);
        let read_used = read + spacing + ATTENTION_ACTION_SIZE * ATTENTION_ACTION_READ_COUNT;
        assert!((read_used - remaining).abs() < f32::EPSILON);
    }

    #[test]
    fn change_tree_nests_directories_and_counts_files() {
        let a = terminator_git::Change {
            path: "/repo/src/a.rs".into(),
            status: " M".into(),
        };
        let b = terminator_git::Change {
            path: "/repo/src/deep/b.rs".into(),
            status: "??".into(),
        };
        let c = terminator_git::Change {
            path: "/repo/root.rs".into(),
            status: " M".into(),
        };
        let entries = vec![&a, &b, &c];
        let tree = build_change_tree(&entries, Some(Path::new("/repo")));
        assert_eq!(tree.files.len(), 1);
        assert_eq!(tree.dirs["src"].files.len(), 1);
        assert_eq!(tree.dirs["src"].dirs["deep"].files.len(), 1);
        assert_eq!(tree.dirs["src"].count_files(), 2);
        assert_eq!(tree.count_files(), 3);
        let mut all = Vec::new();
        tree.collect(&mut all);
        assert_eq!(all.len(), 3);
    }

    #[cfg(feature = "test-support")]
    fn recorded_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp(egui::Id::new(("fixture-target", name))))
    }

    #[cfg(feature = "test-support")]
    fn click(rect: egui::Rect, mut draw: impl FnMut(Vec<egui::Event>)) {
        let pos = rect.center();
        draw(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn git_panel_renders_icon_controls_stats_and_committed_section() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let context = ContextData {
            cwd: "/repo".into(),
            root: Some("/repo".into()),
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![
                terminator_git::Change {
                    path: "/repo/a.rs".into(),
                    status: " M".into(),
                },
                terminator_git::Change {
                    path: "/repo/new.rs".into(),
                    status: "??".into(),
                },
            ],
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::from([(PathBuf::from("/repo/a.rs"), (4, 2))]),
            error: None,
        };
        let compare = workspace_ops::CompareData {
            root: Some("/repo".into()),
            upstream: Some("origin/main".into()),
            ahead: 1,
            behind: 0,
            base: Some("origin/main".into()),
            files: vec![workspace_ops::CommittedFile {
                path: "/repo/lib.rs".into(),
                letter: 'M',
                added: 3,
                deleted: 1,
            }],
        };
        let branches = vec!["main".to_string(), "dev".to_string()];
        let theme = AppearanceConfig::default();
        let mut outcome = GitPanelOutcome::default();
        let draw = |events: Vec<egui::Event>, outcome: &mut GitPanelOutcome| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut draft = String::new();
                    let mut input = GitPanelInput {
                        context: &context,
                        prepared: None,
                        review_mode: ReviewMode::Native,
                        neovim_review: false,
                        theme: &theme,
                        history: false,
                        view_list: false,
                        commits: &[],
                        branches: &branches,
                        commit_draft: &mut draft,
                        collapse_generation: 0,
                        open_shortcut: "",
                        compare: Some(&compare),
                        base_ref: None,
                    };
                    *outcome = git_panel(ui, &mut input);
                },
            );
            output.textures_delta.clear();
        };
        draw(vec![], &mut outcome);
        assert!(recorded_target(&ctx, "git-changes").is_some());
        assert!(recorded_target(&ctx, "git-history").is_some());
        assert!(recorded_target(&ctx, "git-collapse").is_some());
        assert!(recorded_target(&ctx, "git-file-a.rs").is_some());
        assert!(recorded_target(&ctx, "git-file-new.rs").is_some());
        assert!(recorded_target(&ctx, "git-committed-lib.rs").is_some());
        let stage_all = recorded_target(&ctx, "git-stage-all").expect("stage all is painted");
        click(stage_all, |events| draw(events, &mut outcome));
        assert!(outcome.stage_all);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn git_toolbar_shows_inline_actions_when_wide_and_overflow_when_narrow() {
        let make_context = || ContextData {
            cwd: "/repo".into(),
            root: Some("/repo".into()),
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![],
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::default(),
            error: None,
        };
        let compare = workspace_ops::CompareData {
            root: Some("/repo".into()),
            upstream: Some("origin/main".into()),
            ahead: 1,
            behind: 0,
            base: Some("origin/main".into()),
            files: vec![],
        };
        let branches = vec!["main".to_string(), "dev".to_string()];
        let theme = AppearanceConfig::default();
        let context = make_context();
        let draw = |ctx: &egui::Context,
                    width: f32,
                    events: Vec<egui::Event>,
                    outcome: &mut GitPanelOutcome| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut draft = String::new();
                    let mut input = GitPanelInput {
                        context: &context,
                        prepared: None,
                        review_mode: ReviewMode::Native,
                        neovim_review: false,
                        theme: &theme,
                        history: false,
                        view_list: false,
                        commits: &[],
                        branches: &branches,
                        commit_draft: &mut draft,
                        collapse_generation: 0,
                        open_shortcut: "",
                        compare: Some(&compare),
                        base_ref: None,
                    };
                    *outcome = git_panel(ui, &mut input);
                },
            );
            output.textures_delta.clear();
        };
        let wide = egui::Context::default();
        crate::appearance::install(&wide);
        let mut outcome = GitPanelOutcome::default();
        draw(&wide, 360.0, vec![], &mut outcome);
        assert!(recorded_target(&wide, "git-view-list").is_some());
        assert!(recorded_target(&wide, "git-base-ref").is_some());
        assert!(recorded_target(&wide, "git-compare-refresh").is_some());
        assert!(recorded_target(&wide, "git-more").is_none());
        let list = recorded_target(&wide, "git-view-list").expect("list toggle");
        let pos = list.center();
        draw(
            &wide,
            360.0,
            vec![egui::Event::PointerMoved(pos)],
            &mut outcome,
        );
        for pressed in [true, false] {
            draw(
                &wide,
                360.0,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut outcome,
            );
        }
        assert!(outcome.toggle_list);
        let narrow = egui::Context::default();
        crate::appearance::install(&narrow);
        let mut outcome = GitPanelOutcome::default();
        draw(&narrow, 200.0, vec![], &mut outcome);
        assert!(recorded_target(&narrow, "git-more").is_some());
        assert!(recorded_target(&narrow, "git-view-list").is_none());
        assert!(recorded_target(&narrow, "git-base-ref").is_none());
        assert!(recorded_target(&narrow, "git-compare-refresh").is_none());
    }

    #[test]
    fn sidebar_groups_live_sessions_per_workspace_tab_in_layout_order() {
        use super::super::explorer_projects::group_session_ids_by_tab;
        let tabs = vec![
            (
                "tab-a".to_string(),
                vec!["s2".to_string(), "s1".to_string()],
            ),
            ("tab-b".to_string(), vec!["s3".to_string()]),
            ("tab-empty".to_string(), vec!["gone".to_string()]),
        ];
        // Live order differs from layout order; s4 is in no tab (background).
        let live = vec![
            "s1".to_string(),
            "s2".to_string(),
            "s3".to_string(),
            "s4".to_string(),
        ];
        let (groups, overflow) = group_session_ids_by_tab(&tabs, &live);
        // Layout order wins inside each tab; empty tabs are skipped.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "tab-a");
        assert_eq!(groups[0].1, vec!["s2".to_string(), "s1".to_string()]);
        assert_eq!(groups[1].0, "tab-b");
        assert_eq!(groups[1].1, vec!["s3".to_string()]);
        assert_eq!(overflow, vec!["s4".to_string()]);
    }

    #[test]
    fn sidebar_dedupes_a_session_listed_in_two_tabs() {
        use super::super::explorer_projects::group_session_ids_by_tab;
        let tabs = vec![
            ("tab-a".to_string(), vec!["s1".to_string()]),
            (
                "tab-b".to_string(),
                vec!["s1".to_string(), "s2".to_string()],
            ),
        ];
        let live = vec!["s1".to_string(), "s2".to_string()];
        let (groups, overflow) = group_session_ids_by_tab(&tabs, &live);
        assert_eq!(groups[0].1, vec!["s1".to_string()]);
        assert_eq!(groups[1].1, vec!["s2".to_string()]);
        assert!(overflow.is_empty());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn explorer_segmented_switches_to_contents_mode() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut mode = ExplorerSearchMode::Names;
        let draw = |events: Vec<egui::Event>, mode: &mut ExplorerSearchMode| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 80.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| explorer_segmented(ui, mode),
            );
            output.textures_delta.clear();
        };
        draw(vec![], &mut mode);
        let contents = recorded_target(&ctx, "explorer-mode-contents").expect("contents cell");
        click(contents, |events| draw(events, &mut mode));
        assert_eq!(mode, ExplorerSearchMode::Contents);
    }
}
