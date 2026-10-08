use crate::*;

#[cfg(test)]
mod tests {
    use super::*;

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
    fn section_titles_match_fixture_targets() {
        assert_eq!(SettingsSection::Terminal.title(), "Terminal & Editor");
        assert_eq!(SettingsSection::Updates.title(), "Updates");
        assert_eq!(SettingsSection::ALL.len(), 7);
    }

    #[test]
    fn diff_layout_and_close_timeout_keywords_match_field_names() {
        let keywords = SettingsSection::Terminal.keywords();
        assert!(
            keywords.contains("diff"),
            "Terminal section keywords should mention diff"
        );
        assert!(
            keywords.contains("timeout"),
            "Terminal section keywords should mention timeout"
        );
        assert!(
            keywords.contains("split"),
            "Terminal section keywords should mention split"
        );
        assert!(
            keywords.contains("close"),
            "Terminal section keywords should mention close"
        );
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
}
