use super::*;
use crate::{App, Project, State};
use terminator_core::Paths;

fn fixture() -> (App, egui::Context, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.preferences_writable = false;
    app.apply_state(State {
        projects: ["a", "b"]
            .into_iter()
            .map(|id| Project {
                id: id.into(),
                name: id.into(),
                path: std::path::PathBuf::from(format!("/{id}")),
                layout: serde_json::Value::Null,
            })
            .collect(),
        selected_project: Some("a".into()),
        ..Default::default()
    });
    app.selected = Some("a".into());
    app.player = Controller::finished_fixture("a", Some(0));
    (app, ctx, dir)
}

fn render(app: &mut App, ctx: &egui::Context) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| center(app, ui),
    );
    output.textures_delta.clear();
}

#[test]
fn center_paints_every_player_panel() {
    let (mut app, ctx, _dir) = fixture();
    app.player.eq_open = true;
    app.player.pl_open = true;
    render(&mut app, &ctx);
    app.player.radio_mode = true;
    render(&mut app, &ctx);
    app.player.pl_open = false;
    app.player.eq_open = false;
    render(&mut app, &ctx);
}

#[test]
fn center_renders_when_the_selected_project_does_not_own_playback() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("b".into());
    render(&mut app, &ctx);
}

#[test]
fn format_clock_zero_pads_seconds() {
    assert_eq!(format_clock(Duration::from_secs(0)), "0:00");
    assert_eq!(format_clock(Duration::from_secs(65)), "1:05");
    assert_eq!(format_clock(Duration::from_hours(1)), "60:00");
}

#[test]
fn running_time_uses_the_current_playlist_total() {
    let (mut app, _ctx, _dir) = fixture();
    app.player
        .durations
        .insert(std::path::PathBuf::from("/a"), Duration::from_secs(125));
    assert_eq!(running_time(&app), "0:00/2:05");
}

#[test]
fn eq_presets_only_match_their_own_bands() {
    let (mut app, _ctx, _dir) = fixture();
    app.player.eq_preamp = 0.55;
    app.player.eq_bands = eq_bass().bands;
    assert!(eq_matches(&app, &eq_bass()));
    assert!(!eq_matches(&app, &eq_flat()));
    assert!(!eq_matches(&app, &eq_treble()));
}
