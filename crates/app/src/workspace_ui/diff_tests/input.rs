//! Scripted pointer and key input for the live diff editor tests.
use super::*;

pub(super) struct ClickAndTypeInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ctx: &'a egui::Context,
    pub(super) tab: &'a Tab,
    pub(super) target: &'a str,
    pub(super) text: &'a str,
}

pub(super) struct ClickAndKeyInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ctx: &'a egui::Context,
    pub(super) tab: &'a Tab,
    pub(super) target: &'a str,
    pub(super) key: egui::Key,
}

pub(super) struct ClickAndShortcutInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ctx: &'a egui::Context,
    pub(super) tab: &'a Tab,
    pub(super) target: &'a str,
    pub(super) shift: bool,
}

pub(super) fn fixture_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
}

pub(super) fn run_diff_frames(
    app: &mut App,
    ctx: &egui::Context,
    tab: &Tab,
    frames: &[(f64, Vec<egui::Event>)],
) {
    for (time, events) in frames {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                time: Some(*time),
                events: events.clone(),
                ..Default::default()
            },
            |ui| {
                egui::ScrollArea::new([false, false]).show(ui, |ui| {
                    let available = ui.available_rect_before_wrap();
                    ui.expand_to_include_rect(available);
                    app.diff_view(ui, tab);
                });
            },
        );
        output.textures_delta.clear();
    }
}

pub(super) fn click_and_type(input: ClickAndTypeInput<'_>) {
    let ClickAndTypeInput {
        app,
        ctx,
        tab,
        target,
        text,
    } = input;

    // Times keep moving forward across calls within one test.
    let base = ctx.input(|input| input.time);
    run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
    let rect = fixture_target(ctx, target)
        .unwrap_or_else(|| panic!("editor target {target} was not painted"));
    let pos = rect.center();
    let click = |pressed: bool| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    run_diff_frames(
        app,
        ctx,
        tab,
        &[
            (base + 0.3, click(true)),
            (base + 0.4, click(false)),
            (base + 0.45, vec![]),
            (base + 0.5, vec![egui::Event::Text(text.to_owned())]),
            (base + 0.6, vec![]),
            (base + 0.7, vec![]),
        ],
    );
}

pub(super) fn buffer_text(app: &App, tab: &Tab) -> String {
    app.diff_reviews
        .get(&tab.key())
        .and_then(|review| review.buffer.as_ref())
        .map(|buffer| buffer.text.clone())
        .unwrap_or_default()
}

pub(super) fn click_and_key(input: ClickAndKeyInput<'_>) {
    let ClickAndKeyInput {
        app,
        ctx,
        tab,
        target,
        key,
    } = input;

    let base = ctx.input(|input| input.time);
    run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
    let rect = fixture_target(ctx, target)
        .unwrap_or_else(|| panic!("editor target {target} was not painted"));
    let pos = rect.center();
    let click = |pressed: bool| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    run_diff_frames(
        app,
        ctx,
        tab,
        &[
            (base + 0.3, click(true)),
            (base + 0.4, click(false)),
            (base + 0.45, vec![]),
            (
                base + 0.5,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ),
            (base + 0.6, vec![]),
            (base + 0.7, vec![]),
        ],
    );
}

pub(super) fn click_and_shortcut(input: ClickAndShortcutInput<'_>) {
    let ClickAndShortcutInput {
        app,
        ctx,
        tab,
        target,
        shift,
    } = input;

    let base = ctx.input(|input| input.time);
    run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
    let rect = fixture_target(ctx, target)
        .unwrap_or_else(|| panic!("editor target {target} was not painted"));
    let pos = rect.center();
    let click = |pressed: bool| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let mut modifiers = egui::Modifiers::COMMAND;
    modifiers.shift = shift;
    run_diff_frames(
        app,
        ctx,
        tab,
        &[
            (base + 0.3, click(true)),
            (base + 0.4, click(false)),
            (base + 0.45, vec![]),
            (
                base + 0.5,
                vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
            ),
            (base + 0.6, vec![]),
            (base + 0.7, vec![]),
            (base + 0.8, vec![]),
        ],
    );
}
