//! The Alt+Enter popup: language-server actions, then built-in items
//! (`IntelliJ` intentions) that need no server.
use super::*;

/// Alt+Enter items that need no server answer, listed after the
/// server's actions (`IntelliJ` intentions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinAction {
    GotoImplementation,
    FindUsages,
    Rename,
    AddToAgentContext,
}

impl BuiltinAction {
    /// The items for a file; `server` adds the language-server ones.
    pub(crate) fn for_file(server: bool) -> Vec<Self> {
        let mut items = Vec::new();
        if server {
            items.extend([Self::GotoImplementation, Self::FindUsages, Self::Rename]);
        }
        items.push(Self::AddToAgentContext);
        items
    }

    fn label(self) -> &'static str {
        match self {
            Self::GotoImplementation => "Go to implementations",
            Self::FindUsages => "Find usages",
            Self::Rename => "Rename…",
            Self::AddToAgentContext => "Add to agent context",
        }
    }

    fn shortcut(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match (self, mac) {
            (Self::GotoImplementation, true) => "⌥⌘B",
            (Self::GotoImplementation, false) => "Ctrl+Alt+B",
            (Self::FindUsages, true) => "⌥F7",
            (Self::FindUsages, false) => "Alt+F7",
            (Self::Rename, _)
                if !crate::appearance::editor_style()
                    .prefs
                    .editor_jetbrains_keys =>
            {
                "F2"
            }
            (Self::Rename, true) => "⇧F6",
            (Self::Rename, false) => "Shift+F6",
            (Self::AddToAgentContext, _) => "",
        }
    }
}

/// The Alt+Enter popup.
pub(crate) struct ActionsPopup {
    pub path: PathBuf,
    /// Built-in items, shown after the server's actions.
    pub builtins: Vec<BuiltinAction>,
    /// Generation of the in-flight request (`None` once answered).
    pub generation: Option<u64>,
    /// Buffer revision the actions were asked for; a pick after an edit
    /// is refused, since the action ranges point at the older text.
    pub revision: u64,
    pub items: Vec<native_lsp::CodeActionItem>,
    pub selected: usize,
    pub anchor: Option<egui::Rect>,
}

/// What the popup asked for this frame.
pub(crate) enum ActionPick {
    None,
    Close,
    /// A server action, by index into `items`.
    Pick(usize),
    Builtin(BuiltinAction),
}

/// Keys and clicks for the actions popup, painted at the caret: the
/// server's actions first, then the built-in items.
pub(crate) fn actions_ui(ui: &egui::Ui, popup: &mut ActionsPopup) -> ActionPick {
    let total = popup.items.len().saturating_add(popup.builtins.len());
    if total == 0 {
        return ActionPick::None;
    }
    popup.selected = popup.selected.min(total.saturating_sub(1));
    let mut chosen = None;
    let mut pick = ActionPick::None;
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            popup.selected = popup
                .selected
                .saturating_add(1)
                .checked_rem(total)
                .unwrap_or(0);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            popup.selected = popup
                .selected
                .checked_sub(1)
                .unwrap_or(total.saturating_sub(1));
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            chosen = Some(popup.selected);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            pick = ActionPick::Close;
        }
    });
    if let Some(anchor) = popup.anchor {
        egui::Area::new(egui::Id::unique(("native-code-actions", &popup.path)))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| actions_list(ui, popup, &mut chosen));
                });
            });
    }
    match chosen {
        Some(index) if index < popup.items.len() => ActionPick::Pick(index),
        Some(index) => popup
            .builtins
            .get(index.saturating_sub(popup.items.len()))
            .copied()
            .map_or(pick, ActionPick::Builtin),
        None => pick,
    }
}

/// The popup rows; a click sets `chosen` to the row index.
fn actions_list(ui: &mut egui::Ui, popup: &ActionsPopup, chosen: &mut Option<usize>) {
    for (index, item) in popup.items.iter().enumerate() {
        let mut text = egui::RichText::new(&item.title);
        if item.preferred {
            text = text.strong();
        }
        if ui.selectable_label(index == popup.selected, text).clicked() {
            *chosen = Some(index);
        }
    }
    if popup.generation.is_some() {
        ui.weak("Asking the language server…");
    }
    if !popup.items.is_empty() || popup.generation.is_some() {
        ui.separator();
    }
    for (offset, builtin) in popup.builtins.iter().enumerate() {
        let index = popup.items.len().saturating_add(offset);
        let selected = index == popup.selected;
        let row = ui
            .horizontal(|ui| {
                let label = ui.selectable_label(selected, builtin.label());
                let shortcut = builtin.shortcut();
                if !shortcut.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(shortcut);
                    });
                }
                label
            })
            .inner;
        if row.clicked() {
            *chosen = Some(index);
        }
    }
}
