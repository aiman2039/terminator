use super::*;
use crate::app::OpenFileInput;

#[test]
fn background_player_advances_its_own_project_playlist() {
    let (mut app, _, _dir) = fixture();
    app.selected = Some("b".into());
    app.preferences.playlists = vec![crate::preferences::Playlist {
        name: "Default".into(),
        tracks: vec!["/a/one.wav".into(), "/a/two.wav".into()],
    }];
    app.preferences.selected_playlist = "Default".into();
    app.player = player::Controller::finished_fixture("a", Some(0));
    app.poll_player();
    assert_eq!(app.player.project.as_deref(), Some("a"));
    assert_eq!(app.preferences.player_index.get("Default"), Some(&1));
}

#[test]
fn radio_completion_does_not_start_a_playlist() {
    let (mut app, _, _dir) = fixture();
    app.preferences.playlists = vec![crate::preferences::Playlist {
        name: "Default".into(),
        tracks: vec!["/a/one.wav".into(), "/a/two.wav".into()],
    }];
    app.preferences.selected_playlist = "Default".into();
    app.player = player::Controller::finished_fixture("a", None);
    app.poll_player();
    assert!(!app.preferences.player_index.contains_key("Default"));
}

#[test]
fn closing_a_player_tab_does_not_stop_playback() {
    let (mut app, _, _dir) = fixture();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("player-a".into(), Tab::Player);
    app.player = player::Controller::finished_fixture("a", Some(0));
    app.layouts.get_mut("a").unwrap().close("player-a");
    app.reconcile_gui_resources();
    assert_eq!(app.player.project.as_deref(), Some("a"));
}

#[test]
fn radio_next_wraps_bundled_stations() {
    let (mut app, _, _dir) = fixture();
    app.play_station_at("a", 0);
    assert_eq!(app.player.station_index, Some(0));
    app.play_station_offset("a", 1);
    assert_eq!(app.player.station_index, Some(1));
    app.play_station_offset("a", -1);
    assert_eq!(app.player.station_index, Some(0));
    app.play_station_offset("a", -1);
    assert_eq!(
        app.player.station_index,
        Some(player::radio::catalog().len() - 1)
    );
}

#[test]
fn navigation_retains_identity_and_updates_the_originating_project() {
    let (mut app, ctx, _dir) = fixture();
    app.open_browser_url("a", "https://example.com/start", None)
        .unwrap();
    pump_until(&mut app, &ctx, |app| {
        matches!(app.layouts["a"].active_pane(), Some(Tab::Browser { .. }))
    });
    let key = app.layouts["a"].active_pane().unwrap().key();
    app.selected = Some("b".into());
    let target = BrowserTarget::from_http_url("https://example.com/next").unwrap();
    app.apply_browser_navigation(&key, target.clone());
    assert_eq!(app.layouts["a"].active_pane().unwrap().key(), key);
    assert_eq!(app.browser_urls[&key], "https://example.com/next");
    assert!(
        matches!(app.layouts["a"].active_pane(), Some(Tab::Browser { target: current, .. }) if *current == target)
    );
    assert_eq!(
        app.layouts["a"].tabs[0].primary.as_ref().unwrap().key(),
        key
    );
    let tab_id = app.layouts["a"].active.clone();
    app.layouts.get_mut("a").unwrap().close(&tab_id);
    app.reconcile_gui_resources();
    assert!(!app.browser_urls.contains_key(&key));
}

#[test]
fn audio_open_plays_without_a_player_tab_and_keeps_original_project() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.open_file(OpenFileInput {
        path: crate::test_path("/a/song.MP3"),
        line: None,
        split: None,
        external: false,
    });
    app.select_project("b".into());
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(!app.layouts["a"].contains(&Tab::Player));
    assert_eq!(app.player.project.as_deref(), Some("a"));
    assert!(app.state.sessions.is_empty());
    assert!(!requests.try_iter().any(|j|matches!(j,Job::Control(request,_) if matches!(*request,Request::Create { editor:true,.. }))));
    assert_eq!(app.preferences.selected_playlist, "Default");
    assert_eq!(
        app.preferences.selected_tracks(),
        [crate::test_path("/a/song.MP3")].as_slice()
    );
}

#[test]
fn adding_audio_files_appends_to_the_selected_playlist() {
    let (mut app, _, _dir) = fixture();
    app.add_audio_files(vec![
        crate::test_path("/a/one.MP3"),
        crate::test_path("/a/two.flac"),
        crate::test_path("/a/notes.txt"),
    ]);
    assert_eq!(
        app.preferences.selected_tracks(),
        [
            crate::test_path("/a/one.MP3"),
            crate::test_path("/a/two.flac")
        ]
        .as_slice()
    );
    assert_eq!(app.player.project.as_deref(), Some("a"));
    app.add_audio_files(vec![crate::test_path("/a/three.ogg")]);
    assert_eq!(app.preferences.selected_tracks().len(), 3);
    assert_eq!(app.player.project.as_deref(), Some("a"));
}

#[test]
fn opening_player_seeds_sample_tracks_once() {
    let (mut app, _ctx, _dir) = fixture();
    app.open_player();
    assert_eq!(app.preferences.selected_tracks().len(), 3);
    assert!(
        app.preferences
            .selected_tracks()
            .iter()
            .all(|path| path.extension().is_some_and(|ext| ext == "wav"))
    );
    app.preferences
        .selected_tracks_mut()
        .expect("playlist")
        .clear();
    app.open_player();
    assert!(app.preferences.selected_tracks().is_empty());
}

#[test]
fn player_icon_opens_a_global_window_not_a_tab() {
    let (mut app, _, _dir) = fixture();
    app.open_player();
    assert!(app.player_open);
    assert!(!app.layouts["a"].contains(&Tab::Player));
    app.open_player();
    assert!(app.player_open);
    assert_eq!(
        app.layouts["a"]
            .iter_all_tabs()
            .filter(|(_, tab)| matches!(tab, Tab::Player))
            .count(),
        0
    );
}

#[test]
fn leftover_player_tabs_are_stripped_without_stopping_playback() {
    let (mut app, _, _dir) = fixture();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("player-a".into(), Tab::Player);
    app.player = player::Controller::finished_fixture("a", Some(0));
    app.reconcile_gui_resources();
    assert!(!app.layouts["a"].contains(&Tab::Player));
    assert_eq!(app.player.project.as_deref(), Some("a"));
}

#[test]
fn opening_the_player_closes_a_leftover_player_tab() {
    let (mut app, _, _dir) = fixture();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("player-a".into(), Tab::Player);
    app.open_player();
    assert!(app.player_open);
    assert!(!app.layouts["a"].contains(&Tab::Player));
    app.open_player();
    assert!(app.player_open);
}
