use super::super::*;
/// One header tool or the overflow menu: the shared header square.
pub(super) const HEADER_SLOT: f32 = appearance::HEADER_BUTTON;
pub(super) const HEADER_GAP: f32 = 4.0;
/// The right sidebar toggle and a gap on each side of it.
pub(super) const HEADER_TOGGLE_RESERVE: f32 = HEADER_SLOT + 2.0 * HEADER_GAP;

/// Window controls in the header's right cluster. Tool windows have one
/// home, the stripes; Search Everywhere is the header's centre button.
#[derive(Clone, Copy)]
pub(super) enum HeaderAction {
    Player,
    Agents,
    IdeMode,
    Settings,
}

/// Left to right. A narrow right sidebar moves the trailing ones into the
/// overflow menu first, so Player and the Agents bell stay longest.
pub(super) const HEADER_ACTIONS: [HeaderAction; 4] = [
    HeaderAction::Player,
    HeaderAction::Agents,
    HeaderAction::Settings,
    HeaderAction::IdeMode,
];

pub(super) struct HeaderActionView {
    pub(super) label: &'static str,
    pub(super) icon: &'static str,
    /// Fixture geometry name, only recorded with test-support.
    #[cfg(feature = "test-support")]
    pub(super) target: &'static str,
}

pub(super) struct HeaderToolBounds {
    pub(super) budget: f32,
    pub(super) toggle: egui::Rect,
}

pub(super) fn header_row_width(buttons: usize, menu: bool) -> f32 {
    // Header width is a UI coordinate; the button count can exceed the f32 mantissa.
    let mut width = buttons as f32 * HEADER_SLOT;
    if buttons > 1 {
        {
            width += buttons.saturating_sub(1) as f32 * HEADER_GAP;
        }
    }
    if menu {
        if buttons > 0 {
            width += HEADER_GAP;
        }
        width += HEADER_SLOT;
    }
    width
}

/// Project-header icon, same square as [`appearance::framed_icon`].
pub(super) const PROJECT_HEADER_SLOT: f32 = appearance::HEADER_BUTTON;
pub(super) const PROJECT_HEADER_GAP: f32 = 2.0;
/// Shortest name kept beside the sidebar toggle. Narrower than this, the
/// toggle moves into the menu so the name stays readable.
pub(super) const PROJECT_HEADER_NAME_MIN: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ProjectHeaderChrome {
    /// Name on the left; the sidebar toggle against the sidebar's right edge.
    Toggle { name: f32 },
    /// The sidebar toggle (and Check for Updates outside macOS) is in the menu.
    Menu { name: f32 },
}

pub(super) fn project_title_width(ui: &egui::Ui, name: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measured = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    // The strong label is a hair wider than the body measure.
    measured + 2.0
}

/// The name, a gap, and one header square.
pub(super) fn project_header_row(name: f32) -> f32 {
    if name > 0.0 {
        name + PROJECT_HEADER_GAP + PROJECT_HEADER_SLOT
    } else {
        PROJECT_HEADER_SLOT
    }
}

/// Fit the project name against the sidebar toggle. The toggle stays until
/// the name would shrink below [`PROJECT_HEADER_NAME_MIN`]; then it moves
/// into a menu and the name takes what is left.
pub(super) fn project_header_chrome(available: f32, natural: f32) -> ProjectHeaderChrome {
    let available = available.max(0.0);
    let natural = natural.max(0.0);
    let name_min = PROJECT_HEADER_NAME_MIN.min(natural);
    let room = available - PROJECT_HEADER_SLOT - PROJECT_HEADER_GAP;
    if project_header_row(name_min) <= available {
        let name = if project_header_row(natural) <= available {
            natural
        } else {
            room.max(name_min)
        };
        return ProjectHeaderChrome::Toggle { name };
    }
    ProjectHeaderChrome::Menu {
        name: if room <= 0.0 { 0.0 } else { natural.min(room) },
    }
}

/// How many leading header actions fit. The rest go behind the overflow menu.
pub(super) fn header_visible_count(budget: f32, count: usize) -> usize {
    if header_row_width(count, false) <= budget {
        return count;
    }
    let mut visible = 0;
    while visible < count && header_row_width(visible.saturating_add(1), true) <= budget {
        visible = visible.saturating_add(1);
    }
    visible
}

pub(super) fn header_action_view(action: HeaderAction) -> HeaderActionView {
    match action {
        HeaderAction::Player => HeaderActionView {
            label: "Player",
            icon: "AudioLines",
            #[cfg(feature = "test-support")]
            target: "player-chrome",
        },
        HeaderAction::Agents => HeaderActionView {
            label: "Agents",
            icon: "Bell",
            #[cfg(feature = "test-support")]
            target: "left-agent-bar",
        },
        HeaderAction::IdeMode => HeaderActionView {
            label: "IDE mode",
            icon: Action::ToggleIdeMode.icon(),
            #[cfg(feature = "test-support")]
            target: "tool-ide-mode",
        },
        HeaderAction::Settings => HeaderActionView {
            label: Action::OpenSettings.label(),
            icon: Action::OpenSettings.icon(),
            #[cfg(feature = "test-support")]
            target: "settings",
        },
    }
}

pub(super) fn header_tool_bounds(ui: &egui::Ui) -> HeaderToolBounds {
    let max = ui.max_rect();
    HeaderToolBounds {
        budget: (ui.available_width() - HEADER_TOGGLE_RESERVE).max(0.0),
        toggle: egui::Rect::from_min_max(
            egui::pos2(max.right() - HEADER_SLOT - HEADER_GAP, max.top()),
            max.max,
        ),
    }
}

/// Case-insensitive substring filter for the saved-scrollback viewer.
/// An empty query returns every line.
pub fn filter_history_lines<'a>(text: &'a str, query: &str) -> Vec<&'a str> {
    if query.is_empty() {
        return text.lines().collect();
    }
    let needle = query.to_lowercase();
    text.lines()
        .filter(|line| line.to_lowercase().contains(&needle))
        .collect()
}

pub(super) struct ClickEnabledMenuItemInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) enabled: bool,
    pub(super) label: &'a str,
    pub(super) icon: &'a str,
}

pub(super) fn click_enabled_menu_item(input: ClickEnabledMenuItemInput<'_>) -> bool {
    let ClickEnabledMenuItemInput {
        ui,
        enabled,
        label,
        icon,
    } = input;
    let clicked = ui
        .add_enabled_ui(enabled, |ui| {
            appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: "",
            })
        })
        .inner
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Linux title-bar buttons. They stay inside [`App::WINDOW_CONTROL_RESERVE`]
/// so the project header keeps the same width it has beside the macOS traffic lights.
#[cfg(not(target_os = "macos"))]
pub(super) fn paint_window_controls(ui: &egui::Ui, rect: egui::Rect) {
    let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
    let controls = [
        ("×", "Close", egui::ViewportCommand::Close),
        ("−", "Minimize", egui::ViewportCommand::Minimized(true)),
        (
            "□",
            "Maximize",
            egui::ViewportCommand::Maximized(!maximized),
        ),
    ];
    let slot = rect.width() / 3.0;
    let mut x = rect.left();
    for (label, tip, command) in controls {
        let control =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(slot, rect.height()));
        x += slot;
        let response = ui
            .interact(
                control,
                ui.scope_id().with(("window-control", tip)),
                egui::Sense::click(),
            )
            .on_hover_text(tip);
        if response.hovered() {
            ui.painter().rect_filled(
                control,
                appearance::WIDGET_RADIUS,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let color = ui.visuals().text_color();
        let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
        let text = galley.size();
        ui.painter().galley(
            egui::pos2(
                control.center().x - text.x * 0.5,
                control.center().y - text.y * 0.5,
            ),
            galley,
            color,
        );
        if response.clicked() {
            ui.ctx().send_viewport_cmd(command);
        }
    }
}
