use super::*;
use crate::shortcuts::JetBrainsCommand;

/// This platform's keymap chord for `command`.
fn keymap_chord(command: JetBrainsCommand) -> &'static str {
    shortcuts::jetbrains_app_chords()
        .iter()
        .find(|(_, candidate, _)| *candidate == command)
        .map(|(chord, _, _)| *chord)
        .unwrap()
}

fn press(chord: &str) -> egui::Event {
    let chord = shortcuts::parse(chord).unwrap();
    egui::Event::Key {
        key: chord.key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: chord.modifiers,
    }
}

/// One frame through the shortcut dispatcher. Returns how many key
/// presses are left for the panes (a terminal would get them).
fn dispatch(app: &mut App, ctx: &egui::Context, jetbrains: bool, chord: &str) -> usize {
    let mut left = 0;
    let held = shortcuts::parse(chord).unwrap().modifiers;
    let raw = egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(held),
            press(chord),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        app.dispatch_shortcuts(ui.ctx(), jetbrains);
        left = ui.input(|input| {
            input
                .events
                .iter()
                .filter(|event| matches!(event, egui::Event::Key { .. }))
                .count()
        });
    });
    output.textures_delta.clear();
    left
}

/// A stand-in terminal pane that holds keyboard focus.
fn paint_terminal(ui: &mut egui::Ui) {
    let terminal = ui.allocate_response(egui::vec2(80.0, 40.0), egui::Sense::click());
    shortcuts::mark_terminal(ui.ctx(), terminal.id);
    terminal.request_focus();
}

fn focus_terminal(ctx: &egui::Context) {
    let mut output = ctx.run_ui(egui::RawInput::default(), paint_terminal);
    output.textures_delta.clear();
}

/// One frame with Shift `held`: shortcuts first, then the terminal pane,
/// as in the app frame.
fn shift_frame(app: &mut App, ctx: &egui::Context, held: bool, events: Vec<egui::Event>) {
    let modifiers = if held {
        egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::NONE
    };
    let mut raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers)],
        ..Default::default()
    };
    raw.events.extend(events);
    let mut output = ctx.run_ui(raw, |ui| {
        app.dispatch_shortcuts(ui.ctx(), true);
        paint_terminal(ui);
    });
    output.textures_delta.clear();
}

fn tap_shift(app: &mut App, ctx: &egui::Context) {
    shift_frame(app, ctx, true, Vec::new());
    shift_frame(app, ctx, false, Vec::new());
}

#[test]
fn keymap_chords_open_the_palette_with_no_editor_focused() {
    let (mut app, ctx, _dir) = fixture();
    for (command, filter) in [
        (JetBrainsCommand::FindAction, crate::palette::FILTER_ACTIONS),
        (JetBrainsCommand::GotoFile, crate::palette::FILTER_FILES),
    ] {
        assert_eq!(dispatch(&mut app, &ctx, true, keymap_chord(command)), 0);
        assert!(app.palette.open, "{command:?}");
        assert_eq!(app.palette.filters, filter, "{command:?}");
        app.close_palette();
    }
    // With the keymap off, Find Action is not an app key.
    let chord = keymap_chord(JetBrainsCommand::FindAction);
    assert_eq!(dispatch(&mut app, &ctx, false, chord), 1);
    assert!(!app.palette.open);
}

#[test]
fn cmd_e_opens_recent_files_in_every_pane_and_ide_mode_has_its_own_chord() {
    let (mut app, ctx, _dir) = fixture();
    let keys = app.state.settings.keybindings.clone();
    let recent = shortcuts::binding(&keys, Action::RecentFiles.id());
    assert_eq!(recent, "command+E");
    for jetbrains in [true, false] {
        assert_eq!(dispatch(&mut app, &ctx, jetbrains, &recent), 0);
        assert!(app.palette.open);
        assert_eq!(app.palette.filters, crate::palette::FILTER_FILES);
        app.close_palette();
    }
    let ide = app.preferences.ide_mode;
    let toggle = shortcuts::binding(&keys, Action::ToggleIdeMode.id());
    assert_eq!(dispatch(&mut app, &ctx, true, &toggle), 0);
    assert_ne!(app.preferences.ide_mode, ide);
    assert!(!app.palette.open);
}

#[test]
fn keymap_chords_in_a_focused_terminal_never_reach_the_pty() {
    let (mut app, ctx, _dir) = fixture();
    focus_terminal(&ctx);
    assert!(shortcuts::terminal_has_focus(&ctx));
    for &(chord, command, reach) in shortcuts::jetbrains_app_chords() {
        if !reach.applies(false, false) {
            continue;
        }
        focus_terminal(&ctx);
        let left = dispatch(&mut app, &ctx, true, chord);
        if reach.applies(false, true) {
            assert_eq!(left, 0, "{chord} must not reach the PTY");
            assert!(app.palette.open, "{command:?}");
        } else {
            // Linux: the shell keeps Ctrl+E; Ctrl+Shift+A stays Select all.
            assert_eq!(left, 1, "{chord}");
            assert!(!app.palette.open, "{command:?}");
        }
        app.close_palette();
    }
    // Recent files (the Settings binding) works in a terminal everywhere.
    focus_terminal(&ctx);
    assert_eq!(dispatch(&mut app, &ctx, true, "command+E"), 0);
    assert_eq!(app.palette.filters, crate::palette::FILTER_FILES);
}

#[test]
fn keymap_chords_show_the_palette_over_settings_and_the_player() {
    let (mut app, ctx, _dir) = fixture();
    app.open_settings();
    let chord = keymap_chord(JetBrainsCommand::FindAction);
    assert_eq!(dispatch(&mut app, &ctx, true, chord), 0);
    assert!(app.palette.open);
    assert!(!app.settings_open(), "the palette takes the center pane");
    assert!(app.settings_session, "Settings keeps its draft");
    app.close_palette();
    app.open_player();
    assert_eq!(dispatch(&mut app, &ctx, true, "command+E"), 0);
    assert!(app.palette.open && !app.player_open());
    assert_eq!(app.palette.filters, crate::palette::FILTER_FILES);
}

#[test]
fn double_shift_opens_search_everywhere_in_any_pane_but_not_while_typing() {
    let (mut app, ctx, _dir) = fixture();
    focus_terminal(&ctx);
    tap_shift(&mut app, &ctx);
    assert!(shortcuts::terminal_has_focus(&ctx));
    tap_shift(&mut app, &ctx);
    assert!(app.palette.open);
    assert_eq!(app.palette.filters, 0, "Search Everywhere shows every kind");
    app.close_palette();
    // A capital typed between the taps is not a double Shift.
    tap_shift(&mut app, &ctx);
    shift_frame(
        &mut app,
        &ctx,
        true,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            },
            egui::Event::Text("A".into()),
        ],
    );
    shift_frame(&mut app, &ctx, false, Vec::new());
    assert!(!app.palette.open);
}

#[test]
fn vnc_shortcut_capture_still_wins_over_the_keymap() {
    let (mut app, ctx, _dir) = fixture();
    let mut id = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let response = ui.button("Remote focus fixture");
        response.request_focus();
        id = Some(response.id);
    });
    output.textures_delta.clear();
    app.vnc_views.insert(
        "capture".into(),
        crate::vnc_ui::VncViewState {
            focused: true,
            capture_shortcuts: true,
            focus_id: id,
            visible_last_frame: true,
            ..Default::default()
        },
    );
    let chord = keymap_chord(JetBrainsCommand::FindAction);
    assert_eq!(dispatch(&mut app, &ctx, true, chord), 1);
    assert!(!app.palette.open);
}

fn settings_saves(requests: &mpsc::Receiver<Job>) -> Vec<Settings> {
    let mut saves = Vec::new();
    while let Ok(job) = requests.try_recv() {
        if let Job::Control(request, _) = job
            && let Request::Settings(settings) = *request
        {
            saves.push(settings);
        }
    }
    saves
}

fn publish(app: &mut App, keybindings: &[(&str, &str)]) {
    let mut state = app.state.clone();
    state.revision = state.revision.saturating_add(1);
    state.settings = Settings::default();
    for (action, chord) in keybindings {
        state
            .settings
            .keybindings
            .insert((*action).into(), (*chord).into());
    }
    app.apply_state(state);
}

const OLD_DEFAULTS: &[(&str, &str)] = &[
    ("toggle_ide_mode", "command+E"),
    ("recent_files", "command+alt+E"),
];

fn migrating_app() -> (App, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, _ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.preferences_writable = true;
    app.preferences.typography_migrated = true;
    app.preferences.attention_migrated = true;
    (app, dir, requests)
}

#[test]
fn old_default_bindings_are_saved_moved_once_and_later_choices_stay() {
    let (mut app, _dir, requests) = migrating_app();
    publish(&mut app, OLD_DEFAULTS);
    let saves = settings_saves(&requests);
    assert_eq!(saves.len(), 1);
    let moved = &saves[0].keybindings;
    assert_eq!(moved["toggle_ide_mode"], "command+alt+E");
    assert_eq!(moved["recent_files"], "command+E");
    assert!(
        !app.preferences.keymap_migrated,
        "set once the daemon has it"
    );
    // A snapshot from before the save landed does not save again.
    publish(&mut app, OLD_DEFAULTS);
    assert!(settings_saves(&requests).is_empty());
    let applied: Vec<_> = moved
        .iter()
        .map(|(action, chord)| (action.as_str(), chord.as_str()))
        .collect();
    publish(&mut app, &applied);
    assert!(app.preferences.keymap_migrated);
    // Later the user puts Cmd+E back on IDE mode: that choice stays.
    publish(&mut app, OLD_DEFAULTS);
    assert!(settings_saves(&requests).is_empty());
    assert_eq!(
        shortcuts::binding(&app.state.settings.keybindings, "toggle_ide_mode"),
        "command+E"
    );
}

#[test]
fn a_keymap_saved_before_recent_files_existed_still_moves_cmd_e_to_it() {
    let (mut app, _dir, requests) = migrating_app();
    let mut state = app.state.clone();
    state.revision = state.revision.saturating_add(1);
    state.settings = Settings::default();
    let keys = &mut state.settings.keybindings;
    keys.insert("toggle_ide_mode".into(), "command+E".into());
    keys.remove("recent_files");
    app.apply_state(state);
    let saves = settings_saves(&requests);
    assert_eq!(saves.len(), 1);
    let moved = &saves[0].keybindings;
    assert_eq!(moved["toggle_ide_mode"], "command+alt+E");
    assert_eq!(moved["recent_files"], "command+E", "not left unbound");
}

#[test]
fn customized_bindings_need_no_save() {
    let (mut app, _dir, requests) = migrating_app();
    publish(
        &mut app,
        &[
            ("toggle_ide_mode", "command+I"),
            ("recent_files", "command+R"),
        ],
    );
    assert!(settings_saves(&requests).is_empty());
    assert!(app.preferences.keymap_migrated);
}

#[test]
fn keymap_save_waits_for_the_older_settings_migrations() {
    let (mut app, _dir, requests) = migrating_app();
    app.preferences.attention_migrated = false;
    publish(&mut app, OLD_DEFAULTS);
    assert!(settings_saves(&requests).is_empty());
    assert!(!app.preferences.keymap_migrated);
    // The Attention migration finished in this launch: the keymap save
    // waits for the next launch, so it never races that save.
    app.preferences.attention_migrated = true;
    assert!(app.attention_requested.is_some());
    publish(&mut app, OLD_DEFAULTS);
    assert!(settings_saves(&requests).is_empty());
    assert!(!app.preferences.keymap_migrated);
}
