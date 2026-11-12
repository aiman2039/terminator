use super::*;
use crate::app::tests::support::session_fixture;
use crate::workspace::{Workspace, WorkspaceTab};
use egui_dock::DockState;

/// Project `a` with tab `one` (shells `s1`, `s2` and an editor) and tab
/// `two` (shell `s3`); project `b` has shell `s4`.
fn tabs() -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = crate::app::fixture();
    let mut state = app.state.clone();
    state.sessions = ["s1", "s2", "s3", "s4"]
        .into_iter()
        .map(|sid| session_fixture(sid, SessionKind::Shell))
        .chain([session_fixture("ed", SessionKind::Editor)])
        .collect();
    if let Some(s4) = state.sessions.iter_mut().find(|s| s.id == "s4") {
        s4.project_id = "b".into();
    }
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    let group = |id: &str, tabs: Vec<Tab>| WorkspaceTab {
        id: id.into(),
        name: None,
        pinned: false,
        primary: None,
        layout: DockState::new(tabs),
    };
    let mut workspace = Workspace::from_layout(DockState::new(Vec::new()));
    workspace.tabs = vec![
        group(
            "one",
            vec![
                Tab::Terminal("s1".into()),
                Tab::Terminal("s2".into()),
                Tab::Terminal("ed".into()),
            ],
        ),
        group("two", vec![Tab::Terminal("s3".into())]),
    ];
    workspace.active = "one".into();
    app.layouts.insert("a".into(), workspace);
    app.layouts.insert(
        "b".into(),
        Workspace::from_layout(DockState::new(vec![Tab::Terminal("s4".into())])),
    );
    app.selected = Some("a".into());
    (app, ctx, dir)
}

/// A real PTY running `cat`; its line discipline echoes what is written.
fn echo_backend(app: &mut App, ctx: &egui::Context, sid: &str, id: u64) {
    let backend = egui_term::TerminalBackend::new(
        id,
        ctx.clone(),
        app.pty_tx.clone(),
        egui_term::BackendSettings {
            shell: "/bin/cat".into(),
            ..Default::default()
        },
    )
    .unwrap();
    app.backends.insert(sid.to_owned(), backend);
}

fn screen(app: &mut App, sid: &str) -> String {
    let backend = app.backends.get_mut(sid).unwrap();
    backend.sync();
    backend.select_all();
    backend.selectable_content()
}

/// Waits until `sid` shows `text`; false after 5 s.
fn shows(app: &mut App, sid: &str, text: &str) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if screen(app, sid).contains(text) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn broadcast_targets_exactly_the_shells_of_the_tab_and_stops_on_tab_change() {
    let (mut app, _ctx, _dir) = tabs();
    assert!(app.broadcast_targets().is_empty(), "off by default");
    app.toggle_broadcast();
    assert_eq!(app.broadcast_tab(), Some(("a", "one")));
    assert_eq!(app.broadcast_targets(), ["s1", "s2"]);
    for other in ["s3", "s4", "ed"] {
        assert!(!app.broadcasting_to(other), "{other} is not in the tab");
    }
    app.follow_broadcast_tab();
    assert!(app.broadcast_tab().is_some(), "same tab keeps it on");
    if let Some(workspace) = app.layouts.get_mut("a") {
        workspace.active = "two".into();
    }
    app.follow_broadcast_tab();
    assert!(app.broadcast_tab().is_none());
    assert!(app.broadcast_targets().is_empty());
    // Another project is a tab change too.
    if let Some(workspace) = app.layouts.get_mut("a") {
        workspace.active = "one".into();
    }
    app.toggle_broadcast();
    app.selected = Some("b".into());
    app.follow_broadcast_tab();
    assert!(app.broadcast_tab().is_none());
}

#[test]
fn broadcast_sends_typed_keys_to_the_panes_of_the_tab_only() {
    let (mut app, ctx, _dir) = tabs();
    for (id, sid) in ["s1", "s2", "s3", "s4"].into_iter().enumerate() {
        echo_backend(&mut app, &ctx, sid, 950 + id as u64);
    }
    // Off: nothing is copied.
    app.broadcast_typed("s1", vec![b"quiet".to_vec()]);
    app.toggle_broadcast();
    app.broadcast_typed("s1", vec![b"hello".to_vec()]);
    assert!(shows(&mut app, "s2", "hello"), "the other pane of the tab");
    assert!(!screen(&mut app, "s2").contains("quiet"));
    // Broadcast only copies to the others; the source wrote its own keys.
    assert!(!screen(&mut app, "s1").contains("hello"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!screen(&mut app, "s3").contains("hello"), "another tab");
    assert!(!screen(&mut app, "s4").contains("hello"), "another project");
    // A key typed in a pane outside the tab is not copied.
    app.broadcast_typed("s3", vec![b"outside".to_vec()]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!screen(&mut app, "s2").contains("outside"));
}

#[test]
fn multi_line_broadcast_paste_waits_for_the_confirmation() {
    let (mut app, ctx, _dir) = tabs();
    for (id, sid) in ["s1", "s2"].into_iter().enumerate() {
        echo_backend(&mut app, &ctx, sid, 970 + id as u64);
    }
    app.toggle_broadcast();
    app.broadcast_paste("s1", "make\nrm -rf build\n".into());
    assert!(app.pending_broadcast_paste().is_some());
    std::thread::sleep(Duration::from_millis(200));
    for sid in ["s1", "s2"] {
        assert!(!screen(&mut app, sid).contains("make"), "{sid} waits");
    }
    app.answer_broadcast_paste(false);
    assert!(app.pending_broadcast_paste().is_none());
    std::thread::sleep(Duration::from_millis(200));
    assert!(!screen(&mut app, "s2").contains("make"), "Cancel drops it");
    app.broadcast_paste("s1", "ls\npwd".into());
    app.answer_broadcast_paste(true);
    assert!(shows(&mut app, "s1", "pwd"));
    assert!(shows(&mut app, "s2", "pwd"));
    // One line goes at once, to every pane.
    app.broadcast_paste("s2", "echo one".into());
    assert!(app.pending_broadcast_paste().is_none());
    assert!(shows(&mut app, "s1", "echo one"));
}

#[test]
fn esc_twice_stops_broadcast_and_the_second_esc_never_reaches_a_pty() {
    let (mut app, ctx, _dir) = tabs();
    app.toggle_broadcast();
    let esc = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut left = Vec::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![esc.clone()],
                ..Default::default()
            },
            |ui| {
                app.broadcast_pre_input(ui.ctx(), "s1");
                left.push(ui.input(|i| i.key_pressed(egui::Key::Escape)));
            },
        );
        output.textures_delta.clear();
    }
    assert_eq!(
        left,
        [true, false],
        "the first Esc is typed, the second is not"
    );
    assert!(app.broadcast_tab().is_none());
}
