use super::*;

/// Project `a` with tabs g1..g3, selected in the order g2, g3, g1.
fn switcher_app() -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = fixture();
    app.state.sessions = ["s1", "s2", "s3"]
        .into_iter()
        .map(|sid| session_fixture(sid, SessionKind::Shell))
        .collect();
    let tab = |id: &str, sid: &str| crate::workspace::WorkspaceTab {
        id: id.into(),
        name: None,
        pinned: false,
        primary: Some(Tab::Terminal(sid.into())),
        layout: DockState::new(vec![Tab::Terminal(sid.into())]),
    };
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("s1".into())]));
    workspace.tabs = vec![tab("g1", "s1"), tab("g2", "s2"), tab("g3", "s3")];
    workspace.recent_tabs.clear();
    for id in ["g2", "g3", "g1"] {
        workspace.active = id.into();
        workspace.record_active();
    }
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    (app, ctx, dir)
}

fn input(app: &mut App, ctx: &egui::Context, raw: egui::RawInput) {
    let mut output = ctx.run_ui(raw, |ui| app.switcher_input(ui.ctx()));
    output.textures_delta.clear();
}

fn ctrl() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        command: !cfg!(target_os = "macos"),
        ..egui::Modifiers::NONE
    }
}

fn modifiers(state: egui::Modifiers) -> egui::RawInput {
    egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(state)],
        ..Default::default()
    }
}

#[test]
fn ctrl_tab_release_selects_most_recent_other_tab() {
    let (mut app, ctx, _dir) = switcher_app();
    app.step_switcher(false);
    input(&mut app, &ctx, modifiers(ctrl()));
    assert!(app.switcher.is_some());
    assert_eq!(app.layouts["a"].active, "g1");
    input(&mut app, &ctx, modifiers(egui::Modifiers::NONE));
    assert!(app.switcher.is_none());
    assert_eq!(app.layouts["a"].active, "g3");
    assert_eq!(
        app.layouts["a"].recent_tabs.first().map(String::as_str),
        Some("g3")
    );
}

#[test]
fn repeated_tab_steps_further_back_in_history() {
    let (mut app, _ctx, _dir) = switcher_app();
    app.step_switcher(false);
    app.step_switcher(false);
    app.commit_switcher();
    assert_eq!(app.layouts["a"].active, "g2");
}

#[test]
fn escape_cancels_without_switching() {
    let (mut app, ctx, _dir) = switcher_app();
    app.step_switcher(false);
    let mut raw = modifiers(ctrl());
    raw.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: ctrl(),
    });
    input(&mut app, &ctx, raw);
    assert!(app.switcher.is_none());
    assert_eq!(app.layouts["a"].active, "g1");
}

#[test]
fn switcher_selection_is_dropped_after_project_change() {
    let (mut app, _ctx, _dir) = switcher_app();
    app.step_switcher(false);
    app.selected = Some("b".into());
    app.commit_switcher();
    assert!(app.switcher.is_none());
    assert_eq!(app.layouts["a"].active, "g1");
}

#[test]
fn switcher_shortcut_is_the_only_one_allowed_while_open() {
    let (mut app, _ctx, _dir) = switcher_app();
    app.step_switcher(false);
    assert!(app.shortcut_allowed(Action::SwitcherNext));
    assert!(app.shortcut_allowed(Action::SwitcherBack));
    assert!(!app.shortcut_allowed(Action::NewTerminal));
}
