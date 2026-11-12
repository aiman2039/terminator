use crate::*;

thread_local! {
    /// Rows `field_visible` was asked about on this thread.
    static SEEN: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Record a row the open section asks about (tests only).
pub(super) fn seen_row(label: &str, keywords: &str) {
    SEEN.with(|seen| {
        seen.borrow_mut()
            .push((label.to_owned(), keywords.to_owned()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "test-support")]
    fn narrow_settings_keep_navigation_fields_and_footer_inside_the_card() {
        let (mut app, ctx, _dir) = app();
        app.settings_section = SettingsSection::Terminal;
        for width in [340.0, 600.0, 1100.0] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0));
            for _ in 0..2 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| app.settings_center(ui),
                );
                output.textures_delta.clear();
            }
            let card = appearance::center_card_rect(screen);
            for name in [
                "settings-section:Appearance",
                "settings-section:Terminal & Editor",
                "settings-cancel",
            ] {
                let rect = diagnostics::current_rect(&ctx, name).unwrap();
                assert!(card.contains_rect(rect), "width={width}: {name}: {rect:?}");
            }
            let shell = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(egui::Id::unique(("shell-override", "combo")))
                })
                .unwrap();
            assert!(card.contains_rect(shell), "width={width}: {shell:?}");
        }
    }

    #[test]
    fn older_daemons_cannot_edit_unsupported_settings() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        for supported in [false, true] {
            app.state.capabilities = if supported {
                vec![DIFF_CLOSE_SETTINGS_CAPABILITY.into()]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(app.diff_close_settings(ui).enabled(), supported);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn sections_are_grouped_by_subject_with_distinct_icons() {
        let titles: Vec<_> = SettingsSection::ALL.iter().map(|s| s.title()).collect();
        assert_eq!(
            titles,
            [
                "Appearance",
                "Terminal",
                "Editor",
                "Languages",
                "Git & Diff",
                "Browser",
                "Keymap",
                "Notifications",
                "Agents",
                "Remote",
                "History",
                "Updates",
            ]
        );
        let icons: std::collections::HashSet<_> =
            SettingsSection::ALL.iter().map(|s| s.icon()).collect();
        assert_eq!(icons.len(), SettingsSection::ALL.len());
        for section in SettingsSection::ALL {
            assert!(
                crate::icons::source(section.icon()).uri().is_some(),
                "{section:?} uses a bundled icon"
            );
        }
        // The old hooks deep link opens Agents.
        assert_eq!(
            SettingsSection::AgentHooks.canonical(),
            SettingsSection::Agents
        );
        assert_eq!(
            SettingsSection::Editor.fixture_aliases(),
            &["Terminal & Editor"]
        );
    }

    #[test]
    fn git_and_diff_search_finds_the_diff_layout_and_close_timeout() {
        let (mut app, _ctx, _dir) = app();
        for query in ["diff split", "close timeout", "pull requests"] {
            app.settings_search = query.into();
            assert!(app.section_visible(SettingsSection::Git), "{query}");
        }
    }

    #[test]
    fn search_finds_rows_in_any_section_with_words_in_any_order() {
        let (mut app, _ctx, _dir) = app();
        for (query, section) in [
            ("spell", SettingsSection::Languages),
            ("rust", SettingsSection::Languages),
            ("devtools", SettingsSection::Browser),
            ("vnc", SettingsSection::Remote),
            ("autosave", SettingsSection::Editor),
            ("font terminal", SettingsSection::Terminal),
            ("notification sound", SettingsSection::Notifications),
            ("keymap", SettingsSection::Keymap),
            ("open file", SettingsSection::Keymap),
        ] {
            app.settings_search = query.into();
            assert!(app.section_visible(section), "{query} finds {section:?}");
        }
    }

    /// Every row a section paints is in that section's search text, so a
    /// search for a row's label finds its section.
    #[test]
    fn every_painted_row_is_found_by_its_section_search() {
        let (mut app, ctx, _dir) = app();
        app.settings_draft.editor_mode = EditorMode::Native;
        let mut missed = Vec::new();
        for section in SettingsSection::ALL {
            app.settings_section = section;
            app.settings_search.clear();
            SEEN.with(|seen| seen.borrow_mut().clear());
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 4000.0),
                    )),
                    ..Default::default()
                },
                |ui| app.settings_center(ui),
            );
            output.textures_delta.clear();
            let rows = SEEN.with(|seen| std::mem::take(&mut *seen.borrow_mut()));
            for (label, _) in rows {
                app.settings_search = label.clone();
                if !app.section_visible(section) {
                    missed.push(format!("{section:?}: {label}"));
                }
                app.settings_search.clear();
            }
        }
        assert!(
            missed.is_empty(),
            "searches that miss their section: {missed:?}"
        );
    }

    #[test]
    fn a_search_with_no_match_says_so_and_clear_search_restores_the_page() {
        let (mut app, ctx, _dir) = app();
        app.settings_search = "zzqqxx".into();
        let paint = |app: &mut App, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 820.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.settings_center(ui),
            );
            output.textures_delta.clear();
            crate::app::painted_text(&output.shapes)
        };
        paint(&mut app, vec![]);
        let texts = paint(&mut app, vec![]);
        assert!(
            texts
                .iter()
                .any(|(_, text)| text == "No settings match “zzqqxx”."),
            "{texts:?}"
        );
        let clear = texts
            .iter()
            .find(|(_, text)| text == "Clear search")
            .map(|(at, _)| *at)
            .expect("Clear search");
        let press = |pressed| egui::Event::PointerButton {
            pos: clear,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        paint(&mut app, vec![egui::Event::PointerMoved(clear)]);
        paint(&mut app, vec![press(true)]);
        paint(&mut app, vec![press(false)]);
        assert!(app.settings_search.is_empty());
    }

    #[test]
    fn settings_card_leaves_a_clickable_margin() {
        let outer = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let card = crate::appearance::center_card_rect(outer);
        assert!(outer.contains_rect(card));
        assert!(card.width() <= 1100.0);
        assert!(card.min.y > outer.min.y && card.max.y < outer.max.y);
        assert_eq!(card.center(), outer.center());
    }

    #[test]
    fn clicking_outside_the_settings_card_closes_clean_settings() {
        let (mut app, ctx, _dir) = app();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 820.0));
        let frame = |events: Vec<egui::Event>, app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| app.settings_center(ui),
            );
            output.textures_delta.clear();
        };
        frame(vec![], &mut app);
        let pos = egui::pos2(4.0, 410.0);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos), button(true)], &mut app);
        frame(vec![button(false)], &mut app);
        assert!(!app.settings_open());
    }

    fn app() -> (App, egui::Context, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.preferences_writable = false;
        app.open_settings();
        (app, ctx, dir)
    }

    #[test]
    fn every_settings_section_paints_with_a_live_session() {
        let (mut app, ctx, _dir) = app();
        assert!(app.settings_session);
        for section in SettingsSection::ALL {
            app.settings_section = section;
            app.settings_search.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 820.0),
                    )),
                    ..Default::default()
                },
                |ui| app.settings_center(ui),
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn showing_the_shortcuts_page_leaves_settings_unchanged() {
        let (mut app, ctx, _dir) = app();
        app.settings_section = SettingsSection::Keymap;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 820.0),
                )),
                ..Default::default()
            },
            |ui| app.settings_center(ui),
        );
        output.textures_delta.clear();
        assert!(!app.settings_dirty(), "unbound Git actions add nothing");
    }

    #[test]
    fn clearing_the_ntfy_channel_switches_the_toggle_off() {
        let (mut app, ctx, _dir) = app();
        app.settings_section = SettingsSection::Notifications;
        app.settings_draft.ntfy_enabled = true;
        app.settings_draft.ntfy_channel = "phone".into();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 820.0),
                )),
                ..Default::default()
            },
            |ui| app.settings_center(ui),
        );
        output.textures_delta.clear();
        assert!(app.settings_draft.ntfy_enabled);
        // Clearing the channel is the removal gesture: the toggle must
        // switch off so the draft stays valid and Apply keeps working.
        app.settings_draft.ntfy_channel.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 820.0),
                )),
                ..Default::default()
            },
            |ui| app.settings_center(ui),
        );
        output.textures_delta.clear();
        assert!(!app.settings_draft.ntfy_enabled);
        assert!(app.settings_validation().is_ok());
    }

    #[test]
    fn notification_search_hides_unrelated_rows() {
        let (mut app, ctx, _dir) = app();
        app.settings_section = SettingsSection::Notifications;
        app.settings_search = "ntfy".into();
        assert!(!app.field_visible("Place notifications at the side", "position side corner"));
        assert!(!app.field_visible("Terminal notifications", "osc terminal desktop"));
        app.settings_search = "osc".into();
        assert!(app.field_visible("Terminal notifications", "osc terminal desktop"));
        // Searching still paints the section without panicking.
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.settings_center(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn search_narrows_the_visible_sections_to_matches() {
        let (mut app, _ctx, _dir) = app();
        app.settings_search = "notifications".into();
        let visible: Vec<_> = SettingsSection::ALL
            .into_iter()
            .filter(|section| app.section_visible(*section))
            .collect();
        assert!(visible.contains(&SettingsSection::Notifications));
        assert!(visible.len() < SettingsSection::ALL.len());
        app.settings_search = "sparkle".into();
        assert!(app.section_visible(SettingsSection::Updates));
        assert!(!app.section_visible(SettingsSection::Appearance));
    }

    #[test]
    fn a_section_change_keeps_the_draft_without_a_question() {
        let (mut app, _ctx, _dir) = app();
        let saved = app.settings_change_count();
        app.settings_draft.inlay_hints = !app.state.settings.inlay_hints;
        assert_eq!(app.settings_change_count(), saved + 1);
        app.request_settings_section(SettingsSection::Remote);
        assert_eq!(app.settings_section, SettingsSection::Remote);
        assert!(app.settings_pending.is_none(), "no question");
        assert_ne!(
            app.settings_draft.inlay_hints,
            app.state.settings.inlay_hints
        );
        // Discard changes reverts at once; it does not ask either.
        app.discard_settings_draft();
        assert_eq!(app.settings_change_count(), 0);
        assert!(app.settings_pending.is_none());
    }

    #[test]
    fn a_search_lists_matching_rows_of_every_section_with_counts() {
        let (mut app, ctx, _dir) = app();
        app.settings_section = SettingsSection::Appearance;
        app.settings_search = "timeout".into();
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| app.settings_center(ui),
            );
            output.textures_delta.clear();
        }
        // "Editor close timeout" (Git & Diff) and the SSH timeouts (Remote)
        // show together, whichever section was open.
        assert!(app.section_match_count(SettingsSection::Git) >= 1);
        assert!(app.section_match_count(SettingsSection::Remote) >= 2);
    }

    #[test]
    fn follow_os_paints_the_preset_for_the_os_appearance() {
        let (mut app, ctx, _dir) = app();
        app.theme_draft.follow.follow_os_theme = true;
        for (os, preset) in [
            (egui::Theme::Light, AppearanceConfig::intellij_light()),
            (egui::Theme::Dark, AppearanceConfig::carbon()),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    system_theme: Some(os),
                    ..Default::default()
                },
                |ui| app.preview_appearance(ui.ctx()),
            );
            output.textures_delta.clear();
            assert!(app.theme.colors_match(&preset), "{os:?}");
        }
        // The draft keeps its own colors; only the painted theme follows.
        assert!(app.theme_draft.colors_match(&AppearanceConfig::default()));
    }

    #[test]
    fn imported_theme_fills_the_draft_only() {
        let (mut app, ctx, dir) = app();
        let file = dir.path().join("nord.conf");
        std::fs::write(&file, "background = #2e3440\nforeground = #d8dee9\n").unwrap();
        app.start_theme_import(file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.theme_draft.window != "#2E3440" && std::time::Instant::now() < deadline {
            app.settings_section = SettingsSection::Appearance;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.settings_center(ui));
            output.textures_delta.clear();
        }
        assert_eq!(app.theme_draft.window, "#2E3440");
        assert_ne!(
            app.theme_committed.window, "#2E3440",
            "nothing saved before Apply"
        );
    }
}
