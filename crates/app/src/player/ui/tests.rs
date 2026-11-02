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

fn with_list(app: &mut App, name: &str, tracks: &[&str]) {
    app.preferences.create_playlist(name);
    app.player_switch_list(name.into());
    *app.preferences.selected_tracks_mut().unwrap() =
        tracks.iter().map(std::path::PathBuf::from).collect();
}

fn render_events(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| center(app, ui),
    );
    output.textures_delta.clear();
}

fn delete_key() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn running_time_counts_only_the_current_playlist() {
    let (mut app, _ctx, _dir) = fixture();
    with_list(&mut app, "Default", &["/a"]);
    let durations = &mut app.player.durations;
    durations.insert("/a".into(), Duration::from_secs(125));
    durations.insert("/removed".into(), Duration::from_mins(10));
    assert_eq!(running_time(&app), "0:00/2:05");
    with_list(&mut app, "Work", &[]);
    assert_eq!(running_time(&app), "0:00/--:--");
}

#[test]
fn switching_playlist_follows_the_playing_track() {
    let (mut app, _ctx, _dir) = fixture();
    with_list(&mut app, "Work", &["/x", "/three"]);
    with_list(&mut app, "Other", &["/y"]);
    with_list(&mut app, "Default", &["/one", "/two", "/three"]);
    app.player.current_path = Some("/three".into());
    app.player.file_index = Some(2);
    app.player_switch_list("Work".into());
    assert_eq!(app.player.file_index, Some(1));
    app.player_switch_list("Other".into());
    assert_eq!(app.player.file_index, None);
    assert!(!app.player.radio(), "a file still plays, not radio");
}

#[test]
fn delete_key_in_a_text_field_keeps_playlist_tracks() {
    let (mut app, ctx, _dir) = fixture();
    with_list(&mut app, "Default", &["/one", "/two"]);
    app.player.selected.insert(0);
    app.player.stack = Some(Stack::List);
    ctx.memory_mut(|m| m.request_focus(egui::Id::unique("player-playlist-name")));
    render(&mut app, &ctx);
    render_events(&mut app, &ctx, vec![delete_key()]);
    assert_eq!(app.preferences.selected_tracks().len(), 2);
    ctx.memory_mut(egui::Memory::stop_text_input);
    render(&mut app, &ctx);
    render_events(&mut app, &ctx, vec![delete_key()]);
    assert_eq!(app.preferences.selected_tracks().len(), 1);
}

#[test]
fn playlist_delete_waits_for_confirmation() {
    let (mut app, ctx, _dir) = fixture();
    with_list(&mut app, "Default", &["/one"]);
    with_list(&mut app, "Work", &[]);
    app.player.stack = Some(Stack::List);
    app.player.confirm = Some(PlaylistConfirm::DeleteList);
    render(&mut app, &ctx);
    assert!(app.preferences.playlists.iter().any(|l| l.name == "Work"));
}

#[test]
fn custom_stations_dedupe_remove_and_list_their_category() {
    let (mut app, _ctx, _dir) = fixture();
    for _ in 0..2 {
        app.player.station_draft = "https://radio.example/stream".into();
        app.add_custom_station("a");
    }
    assert_eq!(app.preferences.radio_stations.len(), 1);
    assert!(app.radio_categories().iter().any(|c| c == "custom"));
    app.remove_custom_station("https://radio.example/stream");
    assert!(app.preferences.radio_stations.is_empty());
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

#[test]
fn sample_tones_seed_default_even_when_another_playlist_is_selected() {
    let (mut app, _ctx, _dir) = fixture();
    app.preferences.ensure_default_playlist();
    with_list(&mut app, "Work", &["/x"]);
    app.apply_player_samples(vec!["/tone.wav".into()]);
    let default = app
        .preferences
        .playlists
        .iter()
        .find(|list| list.name == "Default")
        .unwrap();
    assert_eq!(default.tracks, [std::path::PathBuf::from("/tone.wav")]);
    assert_eq!(
        app.preferences.selected_tracks(),
        [std::path::PathBuf::from("/x")]
    );
}
