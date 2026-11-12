use super::*;

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One frame: start, then a widget that reads Enter/Esc like a terminal,
/// then the dialog. Returns (answer, keys the "terminal" saw, focused id).
struct Frame {
    answer: Option<ConfirmAnswer>,
    terminal_saw_key: bool,
    focused: Option<egui::Id>,
}

fn frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    dialog: impl Fn() -> Confirm<'static>,
) -> Frame {
    let mut result = Frame {
        answer: None,
        terminal_saw_key: false,
        focused: None,
    };
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let ctx = ui.ctx().clone();
        begin_confirm_frame(&ctx);
        result.terminal_saw_key = ctx.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Escape)
        });
        result.answer = confirm(&ctx, egui::Window::new("Close tab?"), dialog(), |_| {});
        result.focused = ctx.memory(|memory| memory.focused());
    });
    output.textures_delta.clear();
    result
}

fn close_tab() -> Confirm<'static> {
    Confirm::new("6 sessions are running in this tab.", "")
        .destructive(ConfirmButton::new("Terminate 6 sessions"))
        .default_action(ConfirmButton::new("Keep running"))
}

fn remove_worktree() -> Confirm<'static> {
    Confirm::new("Remove the worktree?", "").destructive(ConfirmButton::new("Remove worktree"))
}

#[test]
fn enter_runs_the_default_action_and_never_reaches_a_terminal() {
    let ctx = egui::Context::default();
    assert_eq!(frame(&ctx, Vec::new(), close_tab).answer, None);
    let pressed = frame(&ctx, vec![key(egui::Key::Enter)], close_tab);
    assert_eq!(pressed.answer, Some(ConfirmAnswer::Default));
    assert!(!pressed.terminal_saw_key);
}

#[test]
fn escape_cancels_and_never_reaches_a_terminal() {
    let ctx = egui::Context::default();
    frame(&ctx, Vec::new(), close_tab);
    let pressed = frame(&ctx, vec![key(egui::Key::Escape)], close_tab);
    assert_eq!(pressed.answer, Some(ConfirmAnswer::Cancel));
    assert!(!pressed.terminal_saw_key);
}

#[test]
fn enter_never_runs_the_destructive_action() {
    let ctx = egui::Context::default();
    frame(&ctx, Vec::new(), remove_worktree);
    let pressed = frame(&ctx, vec![key(egui::Key::Enter)], remove_worktree);
    assert_eq!(pressed.answer, Some(ConfirmAnswer::Cancel));
}

#[test]
fn enter_waits_while_the_default_action_is_disabled() {
    let ctx = egui::Context::default();
    let busy = || {
        Confirm::new("First connection to host.", "")
            .default_action(ConfirmButton::new("Trust and connect").disabled(Some("Reading keys")))
    };
    frame(&ctx, Vec::new(), busy);
    assert_eq!(frame(&ctx, vec![key(egui::Key::Enter)], busy).answer, None);
}

#[test]
fn the_default_action_has_keyboard_focus_when_the_dialog_opens() {
    let ctx = egui::Context::default();
    let opened = frame(&ctx, Vec::new(), close_tab);
    assert!(opened.focused.is_some());
    let next = frame(&ctx, Vec::new(), close_tab);
    assert_eq!(next.focused, opened.focused);
}

#[test]
fn keys_pass_through_when_no_confirmation_shows() {
    let ctx = egui::Context::default();
    let mut saw = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::Enter)],
            ..Default::default()
        },
        |ui| {
            begin_confirm_frame(ui.ctx());
            saw = ui.input(|input| input.key_pressed(egui::Key::Enter));
        },
    );
    output.textures_delta.clear();
    assert!(saw);
}
