use super::super::*;
pub(super) const HOVER_POPUP_DELAY: Duration = Duration::from_millis(400);
pub(super) const HEADER_SLOT: f32 = 36.0;
pub(super) const HEADER_GAP: f32 = 4.0;
pub(super) const HEADER_MENU_SLOT: f32 = 32.0;
pub(super) const HEADER_TOGGLE_RESERVE: f32 = 36.0;

#[derive(Clone, Copy)]
pub(super) enum HeaderAction {
    Tool(SidebarTool),
    IdeMode,
    Settings,
    Palette,
}

pub(super) const HEADER_ACTIONS: [HeaderAction; 8] = [
    HeaderAction::Tool(SidebarTool::Explorer),
    HeaderAction::Tool(SidebarTool::Agents),
    HeaderAction::Tool(SidebarTool::Git),
    HeaderAction::Tool(SidebarTool::History),
    HeaderAction::Tool(SidebarTool::Info),
    HeaderAction::Settings,
    HeaderAction::Palette,
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

pub(super) struct HeaderIconButton<'a> {
    pub(super) icon: &'a str,
    pub(super) tip: &'a str,
    pub(super) width: f32,
}

pub(super) fn header_row_width(buttons: usize, menu: bool) -> f32 {
    // Header width is a UI coordinate; the button count can exceed the f32 mantissa.
    #[allow(clippy::cast_precision_loss)]
    let mut width = buttons as f32 * HEADER_SLOT;
    if buttons > 1 {
        #[allow(clippy::cast_precision_loss)]
        {
            width += buttons.saturating_sub(1) as f32 * HEADER_GAP;
        }
    }
    if menu {
        if buttons > 0 {
            width += HEADER_GAP;
        }
        width += HEADER_MENU_SLOT;
    }
    width
}

/// Project-header icon, same square as [`appearance::framed_icon`].
pub(super) const PROJECT_HEADER_SLOT: f32 = 28.0;
pub(super) const PROJECT_HEADER_GAP: f32 = 2.0;
/// Shortest name kept beside Player, Agents, and hide. Narrower than this, those
/// icons move into the menu so the name stays readable.
pub(super) const PROJECT_HEADER_NAME_MIN: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ProjectHeaderChrome {
    /// Name on the left; Player, Agents, and hide packed against the sidebar's right edge.
    Icons { name: f32 },
    /// Player and Agents are in the menu. `hide` puts the sidebar toggle there too.
    Menu { name: f32, hide: bool },
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

pub(super) fn project_header_icons(count: u8) -> f32 {
    match count {
        0 => 0.0,
        1 => PROJECT_HEADER_SLOT,
        2 => PROJECT_HEADER_SLOT + PROJECT_HEADER_GAP + PROJECT_HEADER_SLOT,
        _ => {
            PROJECT_HEADER_SLOT
                + PROJECT_HEADER_GAP
                + PROJECT_HEADER_SLOT
                + PROJECT_HEADER_GAP
                + PROJECT_HEADER_SLOT
        }
    }
}

pub(super) fn project_header_row(icons: u8, name: f32) -> f32 {
    let icons = project_header_icons(icons);
    if name > 0.0 && icons > 0.0 {
        name + PROJECT_HEADER_GAP + icons
    } else {
        name + icons
    }
}

/// Fit the project name against the header icons. Icons stay until the name
/// would shrink below [`PROJECT_HEADER_NAME_MIN`]; then Player and Agents
/// collapse into a menu, and the hide control follows if it still does not fit.
pub(super) fn project_header_chrome(available: f32, natural: f32) -> ProjectHeaderChrome {
    let available = available.max(0.0);
    let natural = natural.max(0.0);
    let name_min = PROJECT_HEADER_NAME_MIN.min(natural);
    if project_header_row(3, name_min) <= available {
        let name = if project_header_row(3, natural) <= available {
            natural
        } else {
            (available - project_header_icons(3) - PROJECT_HEADER_GAP).max(name_min)
        };
        return ProjectHeaderChrome::Icons { name };
    }
    if project_header_row(2, name_min) <= available {
        let room = (available - project_header_icons(2) - PROJECT_HEADER_GAP).max(0.0);
        return ProjectHeaderChrome::Menu {
            name: natural.min(room),
            hide: false,
        };
    }
    let menu = project_header_icons(1);
    let name = if menu + PROJECT_HEADER_GAP >= available {
        0.0
    } else {
        natural.min(available - menu - PROJECT_HEADER_GAP)
    };
    ProjectHeaderChrome::Menu { name, hide: true }
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
        HeaderAction::Tool(SidebarTool::Explorer) => HeaderActionView {
            label: "Explorer",
            icon: "Files",
            #[cfg(feature = "test-support")]
            target: "tool-Explorer",
        },
        HeaderAction::Tool(SidebarTool::Agents) => HeaderActionView {
            label: "Agents",
            icon: "Bell",
            #[cfg(feature = "test-support")]
            target: "tool-Agents",
        },
        HeaderAction::Tool(SidebarTool::Git) => HeaderActionView {
            label: "Git",
            icon: "GitBranch",
            #[cfg(feature = "test-support")]
            target: "tool-Git",
        },
        HeaderAction::Tool(SidebarTool::History) => HeaderActionView {
            label: "History",
            icon: "History",
            #[cfg(feature = "test-support")]
            target: "tool-History",
        },
        HeaderAction::Tool(SidebarTool::Info) => HeaderActionView {
            label: "Info",
            icon: "Info",
            #[cfg(feature = "test-support")]
            target: "tool-Info",
        },
        HeaderAction::IdeMode => HeaderActionView {
            label: "IDE mode",
            icon: "LayoutDashboard",
            #[cfg(feature = "test-support")]
            target: "tool-ide-mode",
        },
        HeaderAction::Settings => HeaderActionView {
            label: "Settings",
            icon: "Settings",
            #[cfg(feature = "test-support")]
            target: "settings",
        },
        HeaderAction::Palette => HeaderActionView {
            label: "Command palette",
            icon: "Search",
            #[cfg(feature = "test-support")]
            target: "palette",
        },
    }
}

pub(super) fn header_tool_bounds(ui: &egui::Ui) -> HeaderToolBounds {
    let max = ui.max_rect();
    HeaderToolBounds {
        budget: (ui.available_width() - HEADER_TOGGLE_RESERVE).max(0.0),
        toggle: egui::Rect::from_min_max(egui::pos2(max.right() - 32.0, max.top()), max.max),
    }
}

pub(super) fn header_icon_button(
    ui: &mut egui::Ui,
    button: HeaderIconButton<'_>,
) -> egui::Response {
    let HeaderIconButton { icon, tip, width } = button;
    ui.add_sized(
        [width, 32.0],
        egui::Button::image(
            egui::Image::new(icons::source(icon))
                .tint(appearance::ICON_COLOR)
                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
        )
        .frame(false),
    )
    .on_hover_text(tip)
}

pub(super) fn should_replace_hover_popup(
    hover: &mut Option<(String, Instant)>,
    key: &str,
    popup_key: Option<&str>,
    delay: Duration,
) -> bool {
    if popup_key.is_some() {
        return false;
    }
    let switched = hover.as_ref().is_none_or(|(old, _)| old != key);
    if switched {
        *hover = Some((key.to_owned(), Instant::now()));
    }
    hover
        .as_ref()
        .is_some_and(|(_, since)| since.elapsed() >= delay)
}

/// One wake for the remaining hover delay. An open popup for this key needs
/// no further frame; pointer motion already repaints when the pointer leaves.
pub(super) fn hover_popup_wake(
    hover: &Option<(String, Instant)>,
    key: &str,
    popup_key: Option<&str>,
    delay: Duration,
) -> Option<Duration> {
    if popup_key.is_some() {
        return None;
    }
    let elapsed = hover
        .as_ref()
        .filter(|(stored, _)| stored == key)
        .map(|(_, since)| since.elapsed())
        .unwrap_or(Duration::ZERO);
    Some(delay.saturating_sub(elapsed))
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

pub(super) fn click_enabled_menu_item(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    icon: &str,
) -> bool {
    let clicked = ui
        .add_enabled_ui(enabled, |ui| appearance::menu_item(ui, label, icon, ""))
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
                ui.id().with(("window-control", tip)),
                egui::Sense::click(),
            )
            .on_hover_text(tip);
        if response.hovered() {
            ui.painter()
                .rect_filled(control, 4.0, ui.visuals().widgets.hovered.bg_fill);
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
