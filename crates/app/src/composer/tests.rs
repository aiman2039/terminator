use super::view::{ComposerAct, text_id};
use super::*;
use crate::Paths;
use eframe::egui;

fn app(dir: &Path) -> App {
    let ctx = egui::Context::default();
    App::with_context(&ctx, Paths::at(dir.into()))
}

fn agent(session: &str, state: AgentState) -> terminator_core::Agent {
    terminator_core::Agent {
        invocation_id: "i".into(),
        session_id: session.into(),
        kind: "claude".into(),
        provider_session_id: None,
        state,
        sequence: None,
        updated: 1,
        resume: None,
        process: None,
    }
}

#[test]
fn only_a_waiting_or_completed_hook_state_offers_the_next_prompt() {
    assert!(queue_ready(Some(AgentState::WaitingInput)));
    assert!(queue_ready(Some(AgentState::Completed)));
    for state in [
        None,
        Some(AgentState::Running),
        Some(AgentState::WaitingPermission),
        Some(AgentState::Failed),
        Some(AgentState::Unknown),
    ] {
        assert!(!queue_ready(state), "{state:?}");
    }
}

#[test]
fn a_queued_prompt_is_never_sent_without_the_click() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.queue_prompt("s", "next step");
    app.state.agents.push(agent("s", AgentState::Running));
    // The agent finishes: the queue stays put; frames send nothing.
    app.state.agents[0].state = AgentState::Completed;
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.composer_dialog(&ui.ctx().clone());
        });
        output.textures_delta.clear();
    }
    assert_eq!(app.composer.queues["s"].len(), 1);
    // The click itself goes through the live-agent check: no terminal for
    // `s` here, so nothing is typed and the prompt stays queued.
    assert!(!app.send_next_queued("s"));
    assert_eq!(app.composer.queues["s"].front().unwrap(), "next step");
    // While the agent works, Send next does nothing at all.
    app.state.agents[0].state = AgentState::Running;
    assert!(!app.send_next_queued("s"));
    assert_eq!(app.composer.queues["s"].len(), 1);
}

#[test]
fn paste_never_carries_an_enter_and_flattens_without_bracketed_paste() {
    let text = "first line\nsecond line\n\n";
    let plain = paste_text(text, false);
    assert_eq!(plain, "first line second line");
    let bytes = egui_term::paste_input(&plain, egui_term::TerminalMode::empty());
    assert!(!bytes.contains(&b'\r') && !bytes.contains(&b'\n'));
    let bracketed = paste_text(text, true);
    assert_eq!(bracketed, "first line\nsecond line");
    let bytes = egui_term::paste_input(&bracketed, egui_term::TerminalMode::BRACKETED_PASTE);
    let end = b"\x1b[201~";
    assert!(bytes.ends_with(end), "nothing follows the paste");
}

#[test]
fn prompt_history_is_per_project_newest_first_and_bounded() {
    let mut all = BTreeMap::new();
    let project = Path::new("/w");
    for n in 0..HISTORY_PER_PROJECT + 5 {
        remember_for(&mut all, project, &format!("prompt {n}"));
    }
    remember_for(&mut all, project, "prompt 3");
    let kept = &all[project];
    assert_eq!(kept.len(), HISTORY_PER_PROJECT);
    assert_eq!(kept[0], "prompt 3", "a reused prompt moves to the top once");
    assert_eq!(kept.iter().filter(|p| *p == "prompt 3").count(), 1);
    remember_for(&mut all, project, &"x".repeat(HISTORY_PROMPT_BYTES + 1));
    assert_eq!(
        all[project][0], "prompt 3",
        "an oversized prompt is not kept"
    );
    for n in 0..HISTORY_PROJECTS + 3 {
        remember_for(&mut all, &PathBuf::from(format!("/p{n}")), "a");
    }
    assert_eq!(all.len(), HISTORY_PROJECTS);
}

#[test]
fn composer_keys_never_reach_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.active_session = Some("s".into());
    let ctx = egui::Context::default();
    let mut open = Composer {
        target: "s".into(),
        focus: true,
        ..Composer::default()
    };
    let _ = frame(&ctx, &mut app, Vec::new(), &mut open);
    let typed = frame(
        &ctx,
        &mut app,
        vec![egui::Event::Text("hello".into())],
        &mut open,
    );
    assert_eq!(typed, ComposerAct::None);
    assert_eq!(open.text, "hello");
    assert!(ctx.memory(|m| m.has_focus(text_id())));
    assert!(
        !app.terminal_has_keyboard(&ctx),
        "the focused terminal does not read keys while the composer has them"
    );
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert_eq!(
        frame(&ctx, &mut app, vec![enter], &mut open),
        ComposerAct::None
    );
    assert_eq!(open.text, "hello\n", "Enter is a line break, never a send");
}

/// One frame of the composer form with `events` as input.
fn frame(
    ctx: &egui::Context,
    app: &mut App,
    events: Vec<egui::Event>,
    open: &mut Composer,
) -> ComposerAct {
    let input = egui::RawInput {
        events,
        ..egui::RawInput::default()
    };
    let mut act = ComposerAct::None;
    let mut output = ctx.run_ui(input, |ui| {
        act = app.composer_form_for_test(ui, open);
    });
    output.textures_delta.clear();
    act
}
