//! Scripted pointer and key input for the live diff editor tests.
use super::*;

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

pub(super) struct RunDiffFramesInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ctx: &'a egui::Context,
    pub(super) tab: &'a Tab,
    pub(super) frames: &'a [(f64, Vec<egui::Event>)],
}

pub(super) fn run_diff_frames(input: RunDiffFramesInput<'_>) {
    let RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames,
    } = input;
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

/// Run frames until the live diff (built on the CPU pool) describes the
/// buffer's current revision.
pub(super) fn settle_live_diff(app: &mut App, ctx: &egui::Context, tab: &Tab) {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(30) {
        let current = app
            .diff_reviews
            .get(&tab.key())
            .and_then(|review| review.buffer.as_ref())
            .is_some_and(|buffer| {
                buffer.live.shown.as_ref().is_some_and(|(at, _)| {
                    *at == terminator_native_edit::doc::Buffer::revision(&buffer.editor.doc)
                })
            });
        if current {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        let time = ctx.input(|input| input.time) + 0.1;
        run_diff_frames(RunDiffFramesInput {
            app,
            ctx,
            tab,
            frames: &[(time, vec![])],
        });
    }
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
    run_diff_frames(RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames: &[(base + 0.1, vec![]), (base + 0.2, vec![])],
    });
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
    run_diff_frames(RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames: &[
            (base + 0.3, click(true)),
            (base + 0.4, click(false)),
            (base + 0.45, vec![]),
            (base + 0.5, key_events(key)),
            (base + 0.6, vec![]),
            (base + 0.7, vec![]),
        ],
    });
}

/// A real keyboard sends the key press and, for a printable key, its text.
fn key_events(key: egui::Key) -> Vec<egui::Event> {
    terminator_native_edit::view::keyboard_events(key, egui::Modifiers::NONE)
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
    run_diff_frames(RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames: &[(base + 0.1, vec![]), (base + 0.2, vec![])],
    });
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
    run_diff_frames(RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames: &[
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
    });
}
