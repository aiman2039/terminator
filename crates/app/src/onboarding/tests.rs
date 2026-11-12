use super::*;
use crate::preferences::UiPreferences;

const LOG: &str = "# Changelog\n\n## Unreleased\n\n- newest\n\n## 1.2.0\n\n- one two\n\n## v1.1.0 (2026-01-01)\n\n- older\n";

#[test]
fn whats_new_shows_the_section_of_this_version() {
    assert_eq!(changelog_section(LOG, "1.2.0"), "- one two");
    assert_eq!(changelog_section(LOG, "1.1.0"), "- older");
    // No section for the version yet: the newest one.
    assert_eq!(changelog_section(LOG, "9.9.9"), "- newest");
    assert_eq!(changelog_section("no headings", "1.0.0"), "");
}

#[test]
fn bundled_changelog_has_a_section_to_show() {
    assert!(!changelog_section(whats_new::CHANGELOG, VERSION).is_empty());
}

#[test]
fn welcome_on_first_run_whats_new_after_an_update() {
    let fresh = UiPreferences::default();
    assert_eq!(start_page(&fresh, 0), StartPage::Welcome);
    assert_eq!(start_page(&fresh, 2), StartPage::Nothing);
    let mut prefs = UiPreferences::default();
    prefs.welcome_seen = true;
    assert_eq!(start_page(&prefs, 0), StartPage::Nothing);
    prefs.last_seen_version = Some("0.0.1".into());
    prefs.setup_completed = true;
    assert_eq!(start_page(&prefs, 3), StartPage::WhatsNew);
    prefs.last_seen_version = Some(VERSION.into());
    assert_eq!(start_page(&prefs, 3), StartPage::Nothing);
}

#[test]
fn welcome_page_launches_no_agent_and_sends_no_daemon_request() {
    let dir = tempfile::tempdir().expect("data dir");
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = false;
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.installed_agents = vec![crate::agent_launch::InstalledAgent {
        kind: "codex".into(),
        program: "codex".into(),
    }];
    app.open_welcome();
    assert_eq!(app.center, CenterPage::Welcome);
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.welcome_center(ui));
        output.textures_delta.clear();
    }
    // Only the hook status probe runs; no terminal, no agent, no install.
    let sent: Vec<Job> = received.try_iter().collect();
    assert!(sent.iter().all(|job| matches!(job, Job::HookStatus)));
    assert!(app.post_create.by_tab.is_empty());
}

#[test]
fn first_start_opens_welcome_once_and_remembers_the_version() {
    let dir = tempfile::tempdir().expect("data dir");
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = true;
    app.apply_state(State::default());
    app.state_loaded = true;
    app.onboarding_tick();
    assert_eq!(app.center, CenterPage::Welcome);
    assert_eq!(app.preferences.last_seen_version.as_deref(), Some(VERSION));
    app.hide_center_overlay();
    app.onboarding_tick();
    assert_eq!(app.center, CenterPage::Workspace, "checked once per run");
}
