//! `JetBrains` keymap app chords, where each one applies, and the
//! terminal-focus check the Linux chords need.
use super::Action;
use eframe::egui;

/// What a `JetBrains` keymap chord opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JetBrainsCommand {
    /// An app action: Recent files, Back, or Forward.
    Run(Action),
    /// Find Action: the palette with only the Actions filter on.
    FindAction,
    /// Go to File: the palette with only the Files filter on.
    GotoFile,
}

impl JetBrainsCommand {
    /// The action whose availability gates this command. The palette
    /// commands follow Search Everywhere.
    pub const fn gate(self) -> Action {
        match self {
            Self::Run(action) => action,
            Self::FindAction | Self::GotoFile => Action::OpenPalette,
        }
    }
}

/// Where a `JetBrains` keymap chord applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Only while a native editor has focus.
    Editor,
    /// Every pane except a focused terminal, which keeps the key: on Linux
    /// Ctrl+E moves to the line end and Ctrl+Shift+A is Select all.
    NotTerminal,
    /// Every pane, a focused terminal included.
    Everywhere,
}

impl Reach {
    /// Whether a chord with this reach applies with this keyboard focus.
    pub const fn applies(self, editor: bool, terminal: bool) -> bool {
        match self {
            Self::Editor => editor,
            Self::NotTerminal => !terminal,
            Self::Everywhere => true,
        }
    }
}

/// The keymap's app chords, checked before the Settings bindings. Shift
/// chords come first: egui ignores an extra Shift. Linux uses the
/// `JetBrains` Linux keymap (Ctrl+Shift+N is Go to File there).
pub fn jetbrains_app_chords() -> &'static [(&'static str, JetBrainsCommand, Reach)] {
    if cfg!(target_os = "macos") {
        &[
            (
                "command+shift+A",
                JetBrainsCommand::FindAction,
                Reach::Everywhere,
            ),
            (
                "command+shift+O",
                JetBrainsCommand::GotoFile,
                Reach::Everywhere,
            ),
            (
                "command+E",
                JetBrainsCommand::Run(Action::RecentFiles),
                Reach::Editor,
            ),
            (
                "command+[",
                JetBrainsCommand::Run(Action::NavigateBack),
                Reach::Editor,
            ),
            (
                "command+]",
                JetBrainsCommand::Run(Action::NavigateForward),
                Reach::Editor,
            ),
        ]
    } else {
        &[
            (
                "ctrl+shift+A",
                JetBrainsCommand::FindAction,
                Reach::NotTerminal,
            ),
            (
                "ctrl+shift+N",
                JetBrainsCommand::GotoFile,
                Reach::Everywhere,
            ),
            (
                "ctrl+E",
                JetBrainsCommand::Run(Action::RecentFiles),
                Reach::NotTerminal,
            ),
        ]
    }
}

/// Temp-data key that marks a terminal widget with the pass that painted it.
const TERMINAL_PANE: &str = "terminal-pane";

/// Note that the widget `id` painted a terminal in this pass, so
/// [`terminal_has_focus`] can tell at the start of the next frame.
pub fn mark_terminal(ctx: &egui::Context, id: egui::Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(id.with(TERMINAL_PANE), pass));
}

fn is_terminal(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|data| data.get_temp::<u64>(id.with(TERMINAL_PANE)))
        .is_some()
}

/// Temp-data key for the widget that had keyboard focus when a pass began.
const FRAME_FOCUS: &str = "frame-start-focus";

/// Note which widget has keyboard focus as this pass begins. Call once per
/// pass, before any pane paints.
pub fn note_frame_focus(ctx: &egui::Context) {
    let key = egui::Id::unique(FRAME_FOCUS);
    let now = ctx.memory(egui::Memory::focused);
    let before = ctx
        .data(|data| data.get_temp::<Option<egui::Id>>(key))
        .flatten();
    let other = now.or(before).is_some_and(|id| !is_terminal(ctx, id));
    ctx.data_mut(|data| {
        data.insert_temp(key, now);
        data.insert_temp(key.with("other"), other);
    });
}

/// A widget that is not a terminal had keyboard focus as this pass began,
/// or lost it as the pass began (Esc). A single-line field also gives up
/// focus on Enter while the key stays in the queue. A terminal that takes
/// the free focus now would read that Enter or Esc, so it waits a frame.
pub fn focus_left_another_widget(ctx: &egui::Context) -> bool {
    let other = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::unique(FRAME_FOCUS).with("other")))
        .unwrap_or(false);
    other
        && !ctx
            .memory(egui::Memory::focused)
            .is_some_and(|id| is_terminal(ctx, id))
}

/// Whether keyboard focus is on a terminal painted in this or the last pass.
pub fn terminal_has_focus(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.memory(egui::Memory::focused).is_some_and(|id| {
        ctx.data(|data| data.get_temp::<u64>(id.with(TERMINAL_PANE)))
            .is_some_and(|painted| painted.saturating_add(1) >= pass)
    })
}
