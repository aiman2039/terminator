//! Workspace and terminal rendering.
use super::*;
use egui_term::TerminalBackend;

fn terminal_theme(theme: &AppearanceConfig) -> (String, String, egui_term::TerminalTheme) {
    (
        theme.terminal_background.clone(),
        theme.terminal_foreground.clone(),
        egui_term::TerminalTheme::new(Box::new(egui_term::ColorPalette {
            background: theme.terminal_background.clone(),
            foreground: theme.terminal_foreground.clone(),
            ..Default::default()
        })),
    )
}

fn cached_terminal_theme(
    cache: &mut Option<(String, String, egui_term::TerminalTheme)>,
    theme: &AppearanceConfig,
) -> egui_term::TerminalTheme {
    let cached = cache.get_or_insert_with(|| terminal_theme(theme));
    if cached.0 != theme.terminal_background || cached.1 != theme.terminal_foreground {
        *cached = terminal_theme(theme);
    }
    cached.2.clone()
}

/// Last non-blank grid rows of a live terminal for the drag ghost, oldest
/// first. Bounded so the floating preview stays small.
/// Longest label before a workspace tab ellipsizes. Chrome around it is fixed.
const TAB_TEXT_MAX: f32 = 160.0;

/// Width of a workspace tab for a measured label. Hugs the text instead of
/// a fixed 220px slot, and always reserves the close icon.
pub(super) fn workspace_tab_width(text_width: f32) -> f32 {
    let text = text_width.clamp(8.0, TAB_TEXT_MAX);
    // 30px to the label, then 4px, a 16px close icon, and 8px of padding.
    30.0 + text + 4.0 + 16.0 + 8.0
}

fn tab_label_width(ui: &egui::Ui, label: &str) -> f32 {
    let mut text = egui::text::LayoutJob::simple(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
        TAB_TEXT_MAX,
    );
    text.wrap.max_rows = 1;
    text.wrap.break_anywhere = true;
    ui.painter().layout_job(text).size().x
}

/// Workspace tab hover tooltip: the label plus the session working
/// directory on a second line when the tab has a live session.
fn tab_tooltip(label: &str, cwd: Option<&std::path::Path>) -> String {
    match cwd {
        Some(cwd) => format!("{label}\n{}", cwd.display()),
        None => label.to_owned(),
    }
}

/// Focused-terminal face for a workspace strip tab.
struct TabFace {
    label: String,
    icon: &'static str,
    sid: Option<String>,
    brand: Option<&'static str>,
    status: Option<(AgentState, &'static str, Color32)>,
}

/// Leading icons for one terminal tab, shared by the main-canvas caption,
/// the IDE strip's native tabs, and their width reservations. The brand is
/// the process-inspection identity and shows before any hook. Hook lifecycle
/// replaces the session-kind glyph once status exists. Plain shells keep the
/// kind glyph so every terminal tab carries an icon.
struct TabLeading {
    brand: Option<&'static str>,
    status: Option<(&'static str, Color32, bool)>,
    kind: Option<&'static str>,
}

impl TabLeading {
    fn width(&self) -> f32 {
        // Status and the kind fallback never co-occur: one slot covers both.
        let icons = usize::from(self.brand.is_some())
            .saturating_add(usize::from(self.status.is_some() || self.kind.is_some()));
        f32::from(u16::try_from(icons).unwrap_or(u16::MAX)) * appearance::TERMINAL_LEADING_SLOT
    }
}

/// Terminal sessions in a top-level tab's split layout.
fn group_terminal_ids(layout: &egui_dock::DockState<Tab>) -> Vec<String> {
    layout
        .iter_all_tabs()
        .filter_map(|(_, tab)| match tab {
            Tab::Terminal(sid) => Some(sid.clone()),
            _ => None,
        })
        .collect()
}

/// Aggregate attention badge text and tooltip breakdown for a workspace tab.
/// Input requests, permission requests, and failures stay distinct.
fn tab_attention_text(counts: super::agent_presence::AttentionCounts) -> Option<(String, String)> {
    if counts.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if counts.input > 0 {
        parts.push(format!("{} input", counts.input));
    }
    if counts.permission > 0 {
        parts.push(format!("{} permission", counts.permission));
    }
    if counts.failed > 0 {
        parts.push(format!("{} failed", counts.failed));
    }
    Some((counts.total().to_string(), parts.join(" · ")))
}

fn snapshot_rows(backend: &TerminalBackend) -> Vec<String> {
    let kept: Vec<String> = backend
        .search_rows()
        .into_iter()
        .map(|row| row.text.trim_end().replace('\t', "  "))
        .map(|line| line.chars().take(64).collect::<String>())
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = kept.len().saturating_sub(8);
    kept.into_iter().skip(start).collect()
}

#[cfg(test)]
mod tab_tooltip_tests {
    use super::tab_tooltip;
    use std::path::Path;

    #[test]
    fn tooltip_appends_the_working_directory() {
        assert_eq!(
            tab_tooltip("shell", Some(Path::new("/repo/proj"))),
            "shell\n/repo/proj"
        );
    }

    #[test]
    fn tooltip_without_a_session_is_just_the_label() {
        assert_eq!(tab_tooltip("shell", None), "shell");
    }
}

#[cfg(test)]
mod tab_width_tests {
    use super::workspace_tab_width;

    #[test]
    fn tab_width_tracks_the_label_and_stays_under_the_old_slot() {
        let short = workspace_tab_width(8.0);
        let longer = workspace_tab_width(120.0);
        let capped = workspace_tab_width(400.0);
        assert!(short < longer);
        assert!(longer < 220.0);
        assert_eq!(capped, workspace_tab_width(160.0));
        assert!(capped < 220.0);
    }
}

struct WorkspaceTabMenu {
    close: bool,
    close_all: bool,
    close_left: bool,
    close_right: bool,
    add_left: bool,
    add_right: bool,
}

struct WorkspaceTabMenuSpec<'a> {
    sid: Option<&'a str>,
    index: usize,
    count: usize,
}

fn split_action(direction: Option<&str>) -> &'static str {
    match direction {
        Some("up") => "split_up",
        Some("down") => "split_down",
        Some("left") => "split_left",
        Some("right") => "split_right",
        _ => "new_terminal",
    }
}

fn click_menu_item(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(ui, label, icon, "").clicked();
    if clicked {
        ui.close();
    }
    clicked
}

const HOVER_POPUP_DELAY: Duration = Duration::from_millis(400);
const HEADER_SLOT: f32 = 36.0;
const HEADER_GAP: f32 = 4.0;
const HEADER_MENU_SLOT: f32 = 32.0;
const HEADER_TOGGLE_RESERVE: f32 = 36.0;

#[derive(Clone, Copy)]
enum HeaderAction {
    Tool(SidebarTool),
    IdeMode,
    Settings,
    Palette,
}

const HEADER_ACTIONS: [HeaderAction; 8] = [
    HeaderAction::Tool(SidebarTool::Explorer),
    HeaderAction::Tool(SidebarTool::Agents),
    HeaderAction::Tool(SidebarTool::Git),
    HeaderAction::Tool(SidebarTool::History),
    HeaderAction::Tool(SidebarTool::Info),
    HeaderAction::Settings,
    HeaderAction::Palette,
    HeaderAction::IdeMode,
];

struct HeaderActionView {
    label: &'static str,
    icon: &'static str,
    /// Fixture geometry name, only recorded with test-support.
    #[cfg(feature = "test-support")]
    target: &'static str,
}

struct HeaderToolBounds {
    budget: f32,
    toggle: egui::Rect,
}

struct HeaderIconButton<'a> {
    icon: &'a str,
    tip: &'a str,
    width: f32,
}

fn header_row_width(buttons: usize, menu: bool) -> f32 {
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

/// How many leading header actions fit. The rest go behind the overflow menu.
fn header_visible_count(budget: f32, count: usize) -> usize {
    if header_row_width(count, false) <= budget {
        return count;
    }
    let mut visible = 0;
    while visible < count && header_row_width(visible.saturating_add(1), true) <= budget {
        visible = visible.saturating_add(1);
    }
    visible
}

fn header_action_view(action: HeaderAction) -> HeaderActionView {
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

fn header_tool_bounds(ui: &egui::Ui) -> HeaderToolBounds {
    let max = ui.max_rect();
    HeaderToolBounds {
        budget: (ui.available_width() - HEADER_TOGGLE_RESERVE).max(0.0),
        toggle: egui::Rect::from_min_max(egui::pos2(max.right() - 32.0, max.top()), max.max),
    }
}

fn header_icon_button(ui: &mut egui::Ui, button: HeaderIconButton<'_>) -> egui::Response {
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

fn should_replace_hover_popup(
    hover: &mut Option<(String, Instant)>,
    key: &str,
    popup_key: Option<&str>,
    delay: Duration,
) -> bool {
    let switched = hover.as_ref().is_none_or(|(old, _)| old != key);
    if switched {
        *hover = Some((key.to_owned(), Instant::now()));
    }
    let ready = hover
        .as_ref()
        .is_some_and(|(_, since)| since.elapsed() >= delay);
    let other_popup = popup_key.is_some_and(|shown| shown != key);
    ready || other_popup
}

/// One wake for the remaining hover delay. An open popup for this key needs
/// no further frame; pointer motion already repaints when the pointer leaves.
fn hover_popup_wake(
    hover: &Option<(String, Instant)>,
    key: &str,
    popup_key: Option<&str>,
    delay: Duration,
) -> Option<Duration> {
    if popup_key == Some(key) {
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

fn click_enabled_menu_item(ui: &mut egui::Ui, enabled: bool, label: &str, icon: &str) -> bool {
    let clicked = ui
        .add_enabled_ui(enabled, |ui| appearance::menu_item(ui, label, icon, ""))
        .inner
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl App {
    pub(super) fn terminal_input_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.settings_open
            && !self.player_open
            && !self.command_dialog_open()
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && !self.search_open
    }

    /// Terminals docked in the IDE strip stay interactive while a center-only
    /// view (Settings, Player, palette, or search) covers the main workspace.
    /// True modals still suspend them.
    pub(super) fn strip_terminal_input_enabled(&self, sid: &str) -> bool {
        !self.picker_active
            && !self.add_project
            && !self.notice_detail_modal_open()
            && (self.close_session.is_none() || self.idle_close_pending.is_some())
            && !self.editor_close_sessions.contains(sid)
            && (self.close_workspace.is_none() || self.idle_close_pending.is_some())
            && !self.rename_blocks_input()
            && !self.open_path
            && self.worktree_remove.is_none()
    }

    /// A pointer press on a terminal. Drops the Explorer name field's keyboard
    /// focus without saving or cancelling that prompt.
    pub(super) fn terminal_pressed(&mut self, ctx: &egui::Context, sid: &str) {
        self.name_prompt_focus = false;
        ctx.memory_mut(|memory| memory.surrender_focus(Self::explorer_name_prompt_id()));
        if let Some(preview) = self.markdown.entries.get_mut(sid) {
            preview.editor_focused = true;
        }
        self.active_session = Some(sid.to_owned());
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
    }

    fn terminal_status_color(&self, session: &Session) -> Option<egui::Color32> {
        let presented = self.present_session(&session.id);
        let theme = &self.theme;
        if let Some(state) = presented.lifecycle {
            return Some(appearance::color(match state {
                AgentState::Running => &theme.status_running,
                AgentState::WaitingInput | AgentState::WaitingPermission => &theme.status_waiting,
                AgentState::Failed => &theme.status_failed,
                AgentState::Completed => &theme.accent,
                _ => &theme.secondary,
            }));
        }
        session
            .lifecycle
            .live()
            .then(|| appearance::color(&theme.status_running))
    }

    pub(super) fn branch_at(&self, cwd: &std::path::Path) -> Option<String> {
        if let Some(metadata) = &self.metadata
            && metadata.cwd == cwd
        {
            return metadata.branch.clone().filter(|branch| !branch.is_empty());
        }
        self.context.as_ref().and_then(|context| {
            (context.cwd == cwd && !context.branch.is_empty()).then(|| context.branch.clone())
        })
    }

    fn show_git_sidebar(&mut self) {
        self.preferences.visible = true;
        self.preferences.tool = SidebarTool::Git;
    }

    fn action_tip(&self, label: &str, action: &str) -> String {
        let keys = self.shortcut_label(action);
        if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        }
    }

    fn header_left_width(&self, total: f32) -> f32 {
        let cap = (total - 300.0).max(80.0);
        if self.preferences.left_visible {
            self.project_width.min(cap)
        } else if cfg!(target_os = "macos") {
            200.0_f32.min(cap)
        } else {
            168.0_f32.min(cap)
        }
    }

    fn sidebar_toggle(&mut self, ui: &mut egui::Ui, right: bool) {
        let (action, icon, label) = if right {
            ("toggle_right_sidebar", "PanelRight", "Toggle right sidebar")
        } else {
            ("toggle_left_sidebar", "PanelLeft", "Toggle sidebar")
        };
        let keys = self.shortcut_label(action);
        let tip = if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        };
        let response = appearance::framed_icon(ui, icon, &tip);
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            if right {
                "toggle-right-sidebar"
            } else {
                "toggle-left-sidebar"
            },
            response.rect,
        );
        if response.clicked() {
            if right {
                self.toggle_right_sidebar();
            } else {
                self.toggle_left_sidebar();
            }
        }
    }

    /// Single drag-band row: traffic lights, project label, and tools.
    /// The tab strip is a separate center panel below the sidebars' top
    /// edge so the sidebars run full height.
    pub(super) fn window_header(&mut self, ui: &mut egui::Ui) {
        self.window_header_row(ui);
    }

    /// Top header row: traffic lights, project label, and tools. The middle
    /// is window drag space; tabs live in [`Self::window_header_tabs`].
    fn window_header_row(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let left = self.header_left_width(rect.width());
        let right = self.preferences.width.min(rect.width() - left - 100.0);
        let left_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + left, rect.bottom()));
        let tools_rect =
            egui::Rect::from_min_max(egui::pos2(rect.right() - right, rect.top()), rect.max);
        let tabs_rect = egui::Rect::from_min_max(
            egui::pos2(left_rect.right(), rect.top() + 4.0),
            egui::pos2(tools_rect.left(), rect.bottom() - 4.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if cfg!(target_os = "macos") {
                    let native = ui
                        .input(|i| i.viewport().native_pixels_per_point)
                        .unwrap_or(ui.ctx().pixels_per_point());
                    ui.add_space(72.0 * native / ui.ctx().pixels_per_point());
                } else {
                    for (label, command) in [
                        ("×", egui::ViewportCommand::Close),
                        ("−", egui::ViewportCommand::Minimized(true)),
                        (
                            "□",
                            egui::ViewportCommand::Maximized(
                                !ui.input(|i| i.viewport().maximized.unwrap_or(false)),
                            ),
                        ),
                    ] {
                        if ui.small_button(label).clicked() {
                            ui.ctx().send_viewport_cmd(command);
                        }
                    }
                }
                let response = ui.add(
                    egui::Label::new(
                        RichText::new(
                            self.selected_project()
                                .map_or("Terminator", |p| p.name.as_str()),
                        )
                        .strong(),
                    )
                    .truncate()
                    .sense(egui::Sense::drag()),
                );
                if response.drag_started() {
                    begin_native_window_gesture(ui.ctx(), egui::ViewportCommand::StartDrag);
                }
                self.sidebar_toggle(ui, false);
                header_drag_space(ui);
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
            ui.set_clip_rect(tabs_rect);
            header_drag_space(ui);
        });
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(tools_rect.shrink2(egui::vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| self.header_tools(ui),
        );
    }

    /// Center-only tab strip below the native drag band, where egui owns
    /// every gesture so tabs can be dragged to reorder. Shown as a top
    /// panel after the sidebars so the strip sits beside them, not above.
    pub(super) fn window_header_tabs(&mut self, ui: &mut egui::Ui) {
        ui.set_clip_rect(ui.max_rect());
        if let Some(project) = self.selected.clone() {
            let mut workspace = self
                .layouts
                .remove(&project)
                .unwrap_or_else(Workspace::empty);
            self.workspace_bar(ui, &project, &mut workspace);
            self.layouts.insert(project, workspace);
            // Deferred native closes (`:q`, `:wq`, `:qa` from the file
            // view) run here: the workspace is checked back in, so the
            // tabs resolve again.
            self.drain_pending_native_close();
        } else {
            header_drag_space(ui);
        }
    }

    fn header_tools(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = HEADER_GAP;
        let HeaderToolBounds { budget, toggle } = header_tool_bounds(ui);
        ui.set_max_width(budget);
        let visible = header_visible_count(budget, HEADER_ACTIONS.len());
        self.paint_visible_header_actions(ui, visible);
        self.paint_header_overflow(ui, visible);
        header_drag_space(ui);
        self.paint_header_toggle(ui, toggle);
    }

    fn paint_visible_header_actions(&mut self, ui: &mut egui::Ui, visible: usize) {
        for action in HEADER_ACTIONS.iter().take(visible).copied() {
            self.paint_header_action(ui, action);
        }
    }

    fn paint_header_overflow(&mut self, ui: &mut egui::Ui, visible: usize) {
        if visible >= HEADER_ACTIONS.len() {
            return;
        }
        let Some(hidden) = HEADER_ACTIONS.get(visible..) else {
            return;
        };
        let response = header_icon_button(
            ui,
            HeaderIconButton {
                icon: "Menu",
                tip: "More",
                width: HEADER_MENU_SLOT,
            },
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "header-overflow", response.rect);
        egui::Popup::menu(&response)
            .style(appearance::menu_style)
            .show(|ui| {
                for action in hidden {
                    if self.header_menu_choice(ui, *action) {
                        ui.close();
                    }
                }
            });
    }

    fn paint_header_toggle(&mut self, ui: &mut egui::Ui, toggle: egui::Rect) {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(toggle)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| self.sidebar_toggle(ui, true),
        );
    }

    fn paint_header_action(&mut self, ui: &mut egui::Ui, action: HeaderAction) {
        let view = header_action_view(action);
        let response = match action {
            HeaderAction::Tool(tool) => self.header_tool_button(ui, tool, &view),
            HeaderAction::IdeMode | HeaderAction::Settings | HeaderAction::Palette => {
                header_icon_button(
                    ui,
                    HeaderIconButton {
                        icon: view.icon,
                        tip: &self.header_tip(action),
                        width: HEADER_SLOT,
                    },
                )
            }
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), view.target, response.rect);
        if response.clicked() {
            self.run_header_action(action);
        }
    }

    fn header_tool_button(
        &mut self,
        ui: &mut egui::Ui,
        tool: SidebarTool,
        view: &HeaderActionView,
    ) -> egui::Response {
        let response =
            appearance::tool_button(ui, tool, view.label, self.header_tool_selected(tool));
        if tool == SidebarTool::Explorer {
            response.on_hover_text(self.explorer_tooltip())
        } else {
            response
        }
    }

    fn header_menu_choice(&mut self, ui: &mut egui::Ui, action: HeaderAction) -> bool {
        let view = header_action_view(action);
        let mark = self.header_menu_mark(action);
        let response = appearance::menu_item(ui, view.label, view.icon, &mark);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), view.target, response.rect);
        if response.clicked() {
            self.run_header_action(action);
            true
        } else {
            false
        }
    }

    fn header_tip(&self, action: HeaderAction) -> String {
        let label = header_action_view(action).label;
        let keys = self.header_menu_mark(action);
        if keys.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({keys})")
        }
    }

    fn header_menu_mark(&self, action: HeaderAction) -> String {
        match action {
            HeaderAction::Tool(tool) if self.header_tool_selected(tool) => "✓".into(),
            HeaderAction::IdeMode if self.preferences.ide_mode => "✓".into(),
            HeaderAction::Settings => self.shortcut_label("open_settings"),
            HeaderAction::Palette => self.shortcut_label("open_palette"),
            HeaderAction::IdeMode => self.shortcut_label("toggle_ide_mode"),
            HeaderAction::Tool(_) => String::new(),
        }
    }

    fn header_tool_selected(&self, tool: SidebarTool) -> bool {
        self.preferences.visible && self.preferences.tool == tool
    }

    fn run_header_action(&mut self, action: HeaderAction) {
        match action {
            HeaderAction::Tool(tool) => self.preferences.toggle(tool),
            HeaderAction::IdeMode => self.toggle_ide_mode(),
            HeaderAction::Settings => self.open_settings(),
            HeaderAction::Palette => self.open_command_palette(),
        }
    }
    /// Workspace strip tab face: the focused terminal's label, kind icon,
    /// stable agent brand, and hook lifecycle status, all from the shared
    /// agent presentation model.
    fn tab_face(&self, primary: Option<&Tab>) -> TabFace {
        match primary {
            Some(Tab::Terminal(sid)) => self
                .state
                .sessions
                .iter()
                .find(|s| &s.id == sid)
                .map(|s| {
                    let presented = self.present_session(sid);
                    TabFace {
                        label: s.label.clone(),
                        icon: if s.kind == SessionKind::Editor {
                            "FileCode"
                        } else {
                            "Terminal"
                        },
                        sid: Some(sid.clone()),
                        brand: presented.brand_icon,
                        status: presented.lifecycle.map(|lifecycle| {
                            (
                                lifecycle,
                                presented.status_icon,
                                super::sidebar_ui::state_color(lifecycle, &self.theme),
                            )
                        }),
                    }
                })
                .unwrap_or(TabFace {
                    label: "Terminal".into(),
                    icon: "Terminal",
                    sid: None,
                    brand: None,
                    status: None,
                }),
            Some(Tab::Diff { path, .. } | Tab::Image { path }) => TabFace {
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                icon: if matches!(primary, Some(Tab::Image { .. })) {
                    "FileImage"
                } else {
                    "FileDiff"
                },
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Browser { target, .. }) => TabFace {
                label: target.title(),
                icon: "FileCode",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::NativeEditor { path }) => TabFace {
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                icon: "FileCode",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Player) => TabFace {
                label: "Player".into(),
                icon: "FileMusic",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::CommitLog { .. }) => TabFace {
                label: "Commit Log".into(),
                icon: "FileDiff",
                sid: None,
                brand: None,
                status: None,
            },
            Some(Tab::Blame { path, .. }) => TabFace {
                label: format!(
                    "Blame {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                icon: "FileDiff",
                sid: None,
                brand: None,
                status: None,
            },
            None => TabFace {
                label: "Workspace".into(),
                icon: "Terminal",
                sid: None,
                brand: None,
                status: None,
            },
        }
    }

    /// Resolved leading icons for a terminal tab. Process inspection supplies
    /// the brand before a hook exists. Hook lifecycle then replaces the
    /// session-kind glyph (spinning while running). Unknown sessions resolve
    /// to no icons.
    fn tab_leading(&self, sid: &str) -> TabLeading {
        let presented = self.present_session(sid);
        let kind = self.state.sessions.iter().find(|s| s.id == sid).map(|s| {
            if s.kind == SessionKind::Editor {
                "FileCode"
            } else {
                "Terminal"
            }
        });
        let status = presented.lifecycle.map(|lifecycle| {
            (
                presented.status_icon,
                super::sidebar_ui::state_color(lifecycle, &self.theme),
                lifecycle == AgentState::Running,
            )
        });
        TabLeading {
            brand: presented.brand_icon,
            kind: status.is_none().then_some(kind).flatten(),
            status,
        }
    }

    pub(super) fn workspace_bar(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        workspace: &mut Workspace,
    ) {
        let mut switch = None;
        let mut close = None;
        let mut close_all = false;
        let mut close_left = None;
        let mut close_right = None;
        let mut add_at = None;
        let current = (project.to_owned(), workspace.active.clone());
        let reveal = self.workspace_visible.as_ref() != Some(&current);
        self.workspace_visible = Some(current);
        let previous_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
        let strip_rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 32.0));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "workspace-strip", strip_rect);
        ui.painter()
            .rect_filled(strip_rect, 0, appearance::color(&self.theme.surface));
        ui.horizontal(|ui| {
            let tab_layout: Vec<(f32, super::agent_presence::AttentionCounts)> = workspace
                .tabs
                .iter()
                .map(|group| {
                    let primary = group
                        .primary
                        .as_ref()
                        .filter(|tab| group.layout.find_tab(tab).is_some())
                        .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab));
                    let face = self.tab_face(primary);
                    let width = workspace_tab_width(tab_label_width(ui, &face.label));
                    let attention = self.cached_tab_attention(&group_terminal_ids(&group.layout));
                    (
                        width
                            + if tab_attention_text(attention).is_some() {
                                22.0
                            } else {
                                0.0
                            },
                        attention,
                    )
                })
                .collect();
            // Gap between tabs is a UI coordinate; the count can exceed the f32 mantissa.
            #[allow(clippy::cast_precision_loss)]
            let gaps = tab_layout.len().saturating_sub(1) as f32;
            let content_width = tab_layout.iter().map(|(width, _)| *width).sum::<f32>() + gaps;
            let width = (ui.available_width() - 38.0).max(40.0);
            let overflow = content_width > width;
            let width = (width - if overflow { 58.0 } else { 0.0 }).max(1.0);
            let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("workspace-tabs", project)));
            let offset =
                egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |s| s.offset.x);
            let mut direction = 0.0;
            if overflow {
                let left = ui
                    .add_enabled(
                        offset > 0.5,
                        egui::Button::new("‹").min_size(egui::vec2(27.0, 30.0)),
                    )
                    .on_hover_text("Scroll tabs left");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "tabs-left", left.rect);
                if left.clicked() {
                    direction = -1.0;
                }
            }
            if overflow && ui.rect_contains_pointer(strip_rect) {
                ui.input_mut(|input| {
                    if !input.modifiers.ctrl && !input.modifiers.command {
                        input.smooth_scroll_delta.x += input.smooth_scroll_delta.y;
                        input.smooth_scroll_delta.y = 0.0;
                    }
                });
            }
            let mut strip_rects: Vec<(String, egui::Rect)> =
                Vec::with_capacity(workspace.tabs.len());
            // Strip tabs are drop targets while a terminal pane is dragged,
            // and drag handles for reordering the tabs themselves. Only
            // match drags from this project.
            let drag_same_project = self.pane_drag.as_ref().is_some_and(|pane| match pane {
                Tab::Terminal(sid) => self
                    .state
                    .sessions
                    .iter()
                    .any(|s| &s.id == sid && s.project_id == project),
                _ => false,
            });
            let mut scroll = egui::ScrollArea::horizontal()
                .id_salt(("workspace-tabs", project))
                .max_width(width)
                .auto_shrink([true, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.horizontal(|ui| {
                        let tab_count = workspace.tabs.len();
                        for (index, group) in workspace.tabs.iter().enumerate() {
                            let primary = group
                                .primary
                                .as_ref()
                                .filter(|tab| group.layout.find_tab(tab).is_some())
                                .or_else(|| {
                                    group.layout.iter_all_tabs().next().map(|(_, tab)| tab)
                                });
                            let face = self.tab_face(primary);
                            let Some((width, attention)) = tab_layout.get(index).copied() else {
                                continue;
                            };
                            let badge = tab_attention_text(attention);
                            let active = workspace.active == group.id;
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, 32.0),
                                egui::Sense::click_and_drag(),
                            );
                            strip_rects.push((group.id.clone(), rect));
                            // Dragging a strip tab reorders the top-level
                            // tabs; pane drags keep their own payload.
                            if response.drag_started()
                                && self.pane_drag.is_none()
                                && self.tab_drag.is_none()
                            {
                                self.tab_drag = Some(group.id.clone());
                            }
                            // Destination preview: hovering a strip tab while
                            // dragging a terminal shows that tab's splits so
                            // the drop can target a specific leaf. Switching
                            // back over the origin tab restores it.
                            let hovering_tab = drag_same_project
                                && ui.input(|i| i.pointer.any_down())
                                && ui
                                    .input(|i| i.pointer.interact_pos())
                                    .is_some_and(|pos| rect.contains(pos));
                            if hovering_tab {
                                if self
                                    .drop_preview_origin
                                    .as_ref()
                                    .is_some_and(|(p, g)| p == project && g == &group.id)
                                {
                                    self.drop_preview_origin = None;
                                } else if self.drop_preview_origin.is_none()
                                    && workspace.active != group.id
                                {
                                    self.drop_preview_origin =
                                        Some((project.to_owned(), workspace.active.clone()));
                                }
                                workspace.active.clone_from(&group.id);
                                ui.painter().rect_stroke(
                                    rect,
                                    0,
                                    egui::Stroke::new(2.0, appearance::color(&self.theme.accent)),
                                    egui::StrokeKind::Inside,
                                );
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                                ui.ctx().request_repaint();
                            }
                            if active && reveal {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if active || response.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    0,
                                    if active {
                                        appearance::color(&self.theme.window)
                                    } else {
                                        appearance::color(&self.theme.hover)
                                    },
                                );
                            }
                            let tint = appearance::color(if active {
                                &self.theme.text
                            } else {
                                &self.theme.secondary
                            });
                            // Focused terminal identity: stable brand glyph
                            // beside the hook lifecycle status.
                            if let Some(brand) = face.brand {
                                appearance::paint_status_icon(
                                    ui,
                                    egui::Rect::from_center_size(
                                        egui::pos2(rect.left() + 10.0, rect.center().y),
                                        egui::vec2(13.0, 13.0),
                                    ),
                                    brand,
                                    appearance::ICON_COLOR,
                                    false,
                                );
                            }
                            let status_center = if face.brand.is_some() {
                                rect.left() + 24.0
                            } else {
                                rect.left() + 16.0
                            };
                            let status_size = if face.brand.is_some() { 13.0 } else { 16.0 };
                            let icon_rect = egui::Rect::from_center_size(
                                egui::pos2(status_center, rect.center().y),
                                egui::vec2(status_size, status_size),
                            );
                            if let Some((state, status_icon, tint)) = face.status {
                                appearance::paint_status_icon(
                                    ui,
                                    icon_rect,
                                    status_icon,
                                    tint,
                                    state == AgentState::Running,
                                );
                            } else {
                                egui::Image::new(icons::source(face.icon))
                                    .tint(appearance::ICON_COLOR)
                                    .paint_at(ui, icon_rect);
                            }
                            let label_x = if face.brand.is_some() { 36.0 } else { 30.0 };
                            let badge_reserve = if badge.is_some() { 22.0 } else { 0.0 };
                            let editing = face
                                .sid
                                .as_ref()
                                .is_some_and(|sid| self.renaming(sid, RenameSurface::Workspace));
                            if editing {
                                if let Some(sid) = &face.sid {
                                    self.inline_rename(
                                        ui,
                                        sid,
                                        RenameSurface::Workspace,
                                        egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x + label_x, rect.min.y + 8.0),
                                            egui::pos2(
                                                rect.max.x - (28.0 + badge_reserve),
                                                rect.max.y - 7.0,
                                            ),
                                        ),
                                    );
                                }
                            } else {
                                let mut text = egui::text::LayoutJob::simple(
                                    face.label.clone(),
                                    egui::FontId::proportional(13.0),
                                    tint,
                                    rect.width() - 58.0 - (label_x - 30.0) - badge_reserve,
                                );
                                text.wrap.max_rows = 1;
                                text.wrap.break_anywhere = true;
                                let galley = ui.painter().layout_job(text);
                                ui.painter().galley(
                                    egui::pos2(
                                        rect.left() + label_x,
                                        rect.center().y - galley.size().y * 0.5,
                                    ),
                                    galley,
                                    tint,
                                );
                            }
                            // Aggregate attention across the tab's terminals.
                            if let Some((count, _)) = &badge {
                                let badge_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.right() - 44.0, rect.center().y - 8.0),
                                    egui::pos2(rect.right() - 26.0, rect.center().y + 8.0),
                                );
                                ui.painter().text(
                                    badge_rect.right_center(),
                                    egui::Align2::RIGHT_CENTER,
                                    count,
                                    egui::FontId::proportional(11.0),
                                    appearance::color(if attention.waiting() > 0 {
                                        &self.theme.status_waiting
                                    } else {
                                        &self.theme.status_failed
                                    }),
                                );
                                #[cfg(feature = "test-support")]
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab-attention:{}", face.label),
                                    badge_rect,
                                );
                            }
                            if active {
                                // Inset from the bottom edge. A stroke centered
                                // on rect.bottom() is clipped away by the strip.
                                let underline = egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0),
                                    egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
                                );
                                ui.painter().rect_filled(
                                    underline,
                                    1.0,
                                    appearance::color(&self.theme.accent),
                                );
                                #[cfg(feature = "test-support")]
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab-underline:{}", face.label),
                                    underline,
                                );
                            }
                            let close_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                egui::vec2(20.0, 24.0),
                            );
                            let close_response = ui
                                .interact(
                                    close_rect,
                                    egui::Id::new(("close-workspace", project, &group.id)),
                                    egui::Sense::click(),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Close tab");
                            if response.hovered() || close_response.hovered() || active {
                                egui::Image::new(icons::source("X"))
                                    .tint(appearance::ICON_COLOR)
                                    .paint_at(
                                        ui,
                                        egui::Rect::from_center_size(
                                            close_rect.center(),
                                            egui::vec2(16.0, 16.0),
                                        ),
                                    );
                            }
                            if close_response.clicked() {
                                close = Some(group.id.clone());
                            }
                            if response.clicked()
                                && !editing
                                && self.pane_drag.is_none()
                                && self.tab_drag.is_none()
                                && !close_rect.contains(
                                    response.interact_pointer_pos().unwrap_or(egui::Pos2::ZERO),
                                )
                            {
                                switch = Some(group.id.clone());
                            }
                            if response.double_clicked()
                                && !editing
                                && self.tab_drag.is_none()
                                && let Some(sid) = &face.sid
                            {
                                self.begin_rename(sid, RenameSurface::Workspace);
                            }
                            response
                                .clone()
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_ui(|ui| {
                                    let mut tooltip = tab_tooltip(
                                        &face.label,
                                        face.sid.as_ref().and_then(|sid| {
                                            self.state
                                                .sessions
                                                .iter()
                                                .find(|s| &s.id == sid)
                                                .map(|s| s.cwd.as_path())
                                        }),
                                    );
                                    if let Some(sid) = &face.sid
                                        && let Some(first) = self
                                            .present_session(sid)
                                            .diagnostics(now())
                                            .lines()
                                            .next()
                                    {
                                        tooltip.push_str(&format!("\n{first}"));
                                    }
                                    if let Some((_, breakdown)) = &badge {
                                        tooltip.push_str(&format!("\nAttention: {breakdown}"));
                                    }
                                    ui.label(tooltip);
                                });
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    true,
                                    active,
                                    &face.label,
                                )
                            });
                            appearance::context_menu(&response, |ui| {
                                let action = self.workspace_tab_menu(
                                    ui,
                                    WorkspaceTabMenuSpec {
                                        sid: face.sid.as_deref(),
                                        index,
                                        count: tab_count,
                                    },
                                );
                                if action.close {
                                    close = Some(group.id.clone());
                                }
                                if action.close_all {
                                    close_all = true;
                                }
                                if action.close_left {
                                    close_left = Some(group.id.clone());
                                }
                                if action.close_right {
                                    close_right = Some(group.id.clone());
                                }
                                if action.add_left {
                                    add_at = Some(index);
                                }
                                if action.add_right {
                                    add_at = Some(index.saturating_add(1));
                                }
                            });
                            #[cfg(feature = "test-support")]
                            {
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-tab:{}", face.label),
                                    rect,
                                );
                                diagnostics::record(
                                    ui.ctx(),
                                    &format!("workspace-close:{}", face.label),
                                    close_rect,
                                );
                            }
                        }
                    });
                });
            if overflow {
                let max_offset = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
                let right = ui
                    .add_enabled(
                        scroll.state.offset.x < max_offset - 0.5,
                        egui::Button::new("›").min_size(egui::vec2(27.0, 30.0)),
                    )
                    .on_hover_text("Scroll tabs right");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "tabs-right", right.rect);
                if right.clicked() {
                    direction = 1.0;
                }
                if direction != 0.0 {
                    scroll.state.offset.x = (scroll.state.offset.x
                        + direction * scroll.inner_rect.width().max(1.0) * 0.8)
                        .clamp(0.0, max_offset);
                    scroll.state.store(ui.ctx(), scroll.id);
                    ui.ctx().request_repaint();
                }
            }
            let plus_response = ui
                .add_sized(
                    [30.0, 30.0],
                    egui::Button::new(RichText::new("+").size(18.0)).frame(false),
                )
                .on_hover_text("New top-level terminal tab");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "workspace-plus", plus_response.rect);
            let plus_rect = plus_response.rect;
            if plus_response.clicked() && self.pane_drag.is_none() {
                self.create(None);
            }
            // Dropping a dragged terminal pane onto a strip tab moves it
            // into that tab (its focused split; the tab content was already
            // previewed on hover); dropping into a gap between tabs, onto
            // "+", or onto empty strip background opens it in a fresh
            // top-level tab at that slot. Dropping a dragged strip tab
            // reorders it to the insertion slot instead.
            let released = ui.input(|i| i.pointer.any_released());
            if let Some(pane) = self.pane_drag.clone()
                && drag_same_project
                && released
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                if let Some(group_id) = Self::strip_interior_tab(&strip_rects, pos) {
                    if workspace.move_pane_to_group(&pane, &group_id) {
                        if let Tab::Terminal(sid) = &pane {
                            self.active_session = Some(sid.clone());
                            self.focus_tab = Some(pane.clone());
                        }
                        self.pane_index = None;
                        switch = None;
                    }
                    self.drop_preview_origin = None;
                    self.end_pane_drag();
                } else {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index {
                        if workspace.move_pane_to_new_group_at(&pane, index).is_some() {
                            if let Tab::Terminal(sid) = &pane {
                                self.active_session = Some(sid.clone());
                                self.focus_tab = Some(pane.clone());
                            }
                            self.pane_index = None;
                            switch = None;
                        }
                        self.drop_preview_origin = None;
                        self.end_pane_drag();
                    }
                }
            }
            // A released tab drag the strip did not consume was dropped
            // outside of it: cancel without moving anything.
            if self.tab_drag.is_some() && released {
                if let (Some(dragged), Some(pos)) = (
                    self.tab_drag.clone(),
                    ui.input(|i| i.pointer.interact_pos()),
                ) && workspace.tabs.iter().any(|tab| tab.id == dragged)
                {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index
                        && workspace.reorder_group(&dragged, index)
                    {
                        workspace.active = dragged;
                        switch = None;
                    }
                }
                self.tab_drag = None;
            }
            if self.tab_drag.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.tab_drag = None;
            }
            // The dock paints the previewed tab's focused leaf at real size
            // while a pane hovers a strip tab interior; hovering a gap,
            // "+", or empty strip background shows the tab ghost instead
            // of the pane snapshot ghost. Both mirror the drop routing.
            let strip_pos = (!released && ui.input(|i| i.pointer.any_down()))
                .then(|| ui.input(|i| i.pointer.interact_pos()))
                .flatten();
            self.strip_tab_hover = drag_same_project
                && strip_pos
                    .is_some_and(|pos| Self::strip_interior_tab(&strip_rects, pos).is_some());
            self.strip_new_tab_hover = drag_same_project
                && strip_pos.is_some_and(|pos| {
                    Self::strip_interior_tab(&strip_rects, pos).is_none()
                        && (plus_rect.contains(pos)
                            || Self::strip_insertion_at(&strip_rect, &strip_rects, pos).is_some())
                });
            // Insertion preview: hovering a strip gap, "+", or empty strip
            // background while dragging a pane paints the slot where the
            // fresh top-level tab will land; dragging a strip tab paints
            // the slot it will reorder into. Mirrors the pane drop wash.
            if !released
                && ui.input(|i| i.pointer.any_down())
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                let accent = appearance::color(&self.theme.accent);
                let pane_gap = drag_same_project
                    && self.pane_drag.is_some()
                    && Self::strip_interior_tab(&strip_rects, pos).is_none();
                let tab_member = self
                    .tab_drag
                    .clone()
                    .is_some_and(|dragged| workspace.tabs.iter().any(|tab| tab.id == dragged));
                if pane_gap || tab_member {
                    let index = if plus_rect.contains(pos) {
                        Some(strip_rects.len())
                    } else {
                        Self::strip_insertion_at(&strip_rect, &strip_rects, pos)
                    };
                    if let Some(index) = index {
                        if plus_rect.contains(pos) {
                            ui.painter().rect_stroke(
                                plus_rect,
                                4,
                                egui::Stroke::new(2.0, accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        Self::paint_strip_insertion(ui, &strip_rect, &strip_rects, index, accent);
                        if let Some(dragged) = self.tab_drag.clone()
                            && let Some((_, rect)) =
                                strip_rects.iter().find(|(id, _)| id == &dragged)
                        {
                            ui.painter().rect_stroke(
                                *rect,
                                0,
                                egui::Stroke::new(2.0, accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                        ui.ctx().request_repaint();
                    }
                }
            }
            if self.pane_drag.is_some() {
                ui.ctx().request_repaint();
            }
            header_drag_space(ui);
        });
        ui.painter().hline(
            strip_rect.x_range(),
            strip_rect.bottom(),
            egui::Stroke::new(1.0, appearance::color(&self.theme.border)),
        );
        ui.add_space(1.0);
        ui.spacing_mut().item_spacing = previous_spacing;
        if let Some(id) = switch {
            if workspace.active != id && self.rename_surface == RenameSurface::Pane {
                self.finish_rename(true);
            }
            workspace.active = id;
            self.hover_popup = None;
            self.hide_center_overlay();
        }
        if let Some(id) = close {
            self.begin_workspace_close_tabs(project, vec![id]);
        }
        if close_all {
            self.begin_workspace_close_tabs(project, workspace.ids());
        }
        if let Some(id) = close_left {
            self.begin_workspace_close_tabs(project, workspace.ids_before(&id));
        }
        if let Some(id) = close_right {
            self.begin_workspace_close_tabs(project, workspace.ids_after(&id));
        }
        if let Some(index) = add_at {
            self.create_workspace_tab(Some(index));
        }
    }

    /// Insertion slot for a strip pointer: how many tab centers sit left
    /// of it. None when the pointer leaves the strip band vertically.
    fn strip_insertion_at(
        strip: &egui::Rect,
        tabs: &[(String, egui::Rect)],
        pos: egui::Pos2,
    ) -> Option<usize> {
        if !(strip.top()..=strip.bottom()).contains(&pos.y) {
            return None;
        }
        Some(
            tabs.iter()
                .filter(|(_, rect)| rect.center().x < pos.x)
                .count(),
        )
    }

    /// Strip tab whose interior holds the pointer. Bands near either edge
    /// count as gaps so panes can be dropped between tabs for a positional
    /// new tab instead of landing inside the neighbor.
    fn strip_interior_tab(tabs: &[(String, egui::Rect)], pos: egui::Pos2) -> Option<String> {
        const EDGE: f32 = 12.0;
        tabs.iter()
            .find(|(_, rect)| {
                rect.contains(pos) && pos.x - rect.left() > EDGE && rect.right() - pos.x > EDGE
            })
            .map(|(id, _)| id.clone())
    }

    /// Accent insertion bar marking where a strip drop will land: the gap
    /// before the first tab, between two tabs, or after the last one.
    fn paint_strip_insertion(
        ui: &mut egui::Ui,
        strip: &egui::Rect,
        tabs: &[(String, egui::Rect)],
        index: usize,
        accent: egui::Color32,
    ) {
        let x = if tabs.is_empty() {
            strip.left() + 2.0
        } else if index == 0 {
            tabs.first()
                .map(|tab| tab.1.left())
                .unwrap_or_else(|| strip.left())
        } else if index >= tabs.len() {
            tabs.last()
                .map(|tab| tab.1.right())
                .unwrap_or_else(|| strip.left())
        } else {
            match (tabs.get(index.saturating_sub(1)), tabs.get(index)) {
                (Some(prev), Some(next)) => (prev.1.right() + next.1.left()) * 0.5,
                _ => strip.left(),
            }
        };
        ui.painter().line_segment(
            [
                egui::pos2(x, strip.top() + 4.0),
                egui::pos2(x, strip.bottom() - 4.0),
            ],
            egui::Stroke::new(2.5, accent),
        );
        ui.painter()
            .circle_filled(egui::pos2(x, strip.top() + 4.0), 3.5, accent);
    }

    /// Short label for a dragged tab ghost.
    fn drag_title(&self, tab: &Tab) -> String {
        match tab {
            Tab::Terminal(sid) => self
                .state
                .sessions
                .iter()
                .find(|s| &s.id == sid)
                .map(|s| s.label.clone())
                .unwrap_or_else(|| "Terminal".into()),
            Tab::Diff { path, .. } | Tab::Image { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Tab::NativeEditor { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Tab::Browser { target, .. } => target.title(),
            Tab::Player => "Player".into(),
            Tab::CommitLog { .. } => "Commit Log".into(),
            Tab::Blame { path, .. } => format!(
                "Blame {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
        }
    }

    /// Semi-transparent floating preview following the cursor while a
    /// terminal pane is dragged: its title plus a snapshot of its last grid
    /// rows. Painted on the tooltip layer so it floats above splits and
    /// sidebars without intercepting input.
    pub(super) fn paint_drag_ghost(&self, ui: &mut egui::Ui) {
        let Some(pane) = &self.pane_drag else {
            return;
        };
        // The tab ghost owns the pointer over strip new-tab zones so the
        // two never stack.
        if self.strip_new_tab_hover {
            return;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos())) else {
            return;
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("drag-ghost"),
        ));
        let mut job = egui::text::LayoutJob::simple(
            self.drag_title(pane),
            egui::FontId::proportional(13.0),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            320.0,
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        if !self.pane_drag_snapshot.is_empty() {
            job.wrap.max_rows = 1usize.saturating_add(self.pane_drag_snapshot.len().min(8));
            job.append(
                &format!("\n{}", self.pane_drag_snapshot.join("\n")),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(11.0),
                    color: egui::Color32::from_rgba_unmultiplied(220, 220, 228, 200),
                    ..Default::default()
                },
            );
        }
        let galley = painter.layout_job(job);
        let padding = egui::vec2(12.0, 7.0);
        let size = egui::vec2(
            galley.size().x + padding.x * 2.0,
            galley.size().y + padding.y * 2.0,
        );
        let screen = ui.ctx().content_rect();
        let mut min = egui::pos2(pos.x + 16.0, pos.y + 20.0);
        min.x = min.x.clamp(
            screen.min.x + 4.0,
            (screen.max.x - size.x - 4.0).max(screen.min.x),
        );
        min.y = min.y.clamp(
            screen.min.y + 4.0,
            (screen.max.y - size.y - 4.0).max(screen.min.y),
        );
        let rect = egui::Rect::from_min_size(min, size);
        painter.rect_filled(
            rect,
            6.0,
            egui::Color32::from_rgba_unmultiplied(24, 24, 28, 205),
        );
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.5, appearance::color(&self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(rect.min.x + padding.x, rect.min.y + padding.y),
            galley,
            egui::Color32::WHITE,
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-ghost", rect);
    }

    /// Tab-sized ghost following the cursor while a strip tab is dragged to
    /// reorder: the dragged tab's title at real tab size. Painted on the
    /// tooltip layer like the pane ghost, without intercepting input.
    pub(super) fn paint_tab_ghost(&self, ui: &mut egui::Ui) {
        // Tab reorder drags always show the tab ghost; pane drags show it
        // over strip new-tab zones, where the outcome is a fresh tab.
        if self.tab_drag.is_none() && !(self.pane_drag.is_some() && self.strip_new_tab_hover) {
            return;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos())) else {
            return;
        };
        let title = if let Some(dragged) = &self.tab_drag {
            let mut title = "Tab".to_owned();
            for workspace in self.layouts.values() {
                if let Some(group) = workspace.tabs.iter().find(|tab| &tab.id == dragged) {
                    let pane = group
                        .primary
                        .as_ref()
                        .filter(|tab| group.layout.find_tab(tab).is_some())
                        .or_else(|| group.layout.iter_all_tabs().next().map(|(_, tab)| tab));
                    if let Some(pane) = pane {
                        title = self.drag_title(pane);
                    }
                    break;
                }
            }
            title
        } else if let Some(pane) = &self.pane_drag {
            self.drag_title(pane)
        } else {
            return;
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("tab-ghost"),
        ));
        let mut job = egui::text::LayoutJob::simple(
            title,
            egui::FontId::proportional(13.0),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            TAB_TEXT_MAX,
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = painter.layout_job(job);
        let size = egui::vec2(workspace_tab_width(galley.size().x), 32.0);
        let screen = ui.ctx().content_rect();
        let mut min = egui::pos2(pos.x - size.x * 0.5, pos.y - size.y * 0.5);
        min.x = min.x.clamp(
            screen.min.x + 4.0,
            (screen.max.x - size.x - 4.0).max(screen.min.x),
        );
        min.y = min.y.clamp(
            screen.min.y + 4.0,
            (screen.max.y - size.y - 4.0).max(screen.min.y),
        );
        let rect = egui::Rect::from_min_size(min, size);
        painter.rect_filled(
            rect,
            4.0,
            egui::Color32::from_rgba_unmultiplied(24, 24, 28, 205),
        );
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.5, appearance::color(&self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y * 0.5),
            galley,
            egui::Color32::WHITE,
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "tab-ghost", rect);
    }

    fn workspace_tab_menu(
        &mut self,
        ui: &mut egui::Ui,
        spec: WorkspaceTabMenuSpec<'_>,
    ) -> WorkspaceTabMenu {
        let WorkspaceTabMenuSpec { sid, index, count } = spec;
        if let Some(sid) = sid {
            self.rename_action(ui, sid, RenameSurface::Workspace);
        }
        let close = click_menu_item(ui, "Close tab…", "X");
        ui.separator();
        let close_all = click_enabled_menu_item(ui, count > 1, "Close all tabs…", "X");
        let close_left = click_enabled_menu_item(ui, index > 0, "Close all tabs to the left…", "X");
        let close_right = click_enabled_menu_item(
            ui,
            index.saturating_add(1) < count,
            "Close all tabs to the right…",
            "X",
        );
        ui.separator();
        let add_left = click_menu_item(ui, "Add tab to the left", "Plus");
        let add_right = click_menu_item(ui, "Add tab to the right", "Plus");
        WorkspaceTabMenu {
            close,
            close_all,
            close_left,
            close_right,
            add_left,
            add_right,
        }
    }
    fn new_terminal_menu(
        &mut self,
        ui: &mut egui::Ui,
        pane: Option<egui_dock::NodePath>,
        strip: bool,
    ) {
        for (label, split) in [
            ("New tab", None),
            ("Split up", Some("up")),
            ("Split down", Some("down")),
            ("Split left", Some("left")),
            ("Split right", Some("right")),
        ] {
            let action = split_action(split);
            let icon = match split {
                Some("up") => "PanelTopClose",
                Some("down") => "PanelBottomClose",
                Some("left") => "PanelLeftClose",
                Some("right") => "PanelRightClose",
                _ => "Plus",
            };
            let shortcut = self.shortcut_label(action);
            if appearance::menu_item(ui, label, icon, &shortcut).clicked() {
                if strip {
                    if let Some(pane) = pane {
                        self.add_strip_tab = Some((pane, split.map(str::to_owned)));
                    } else {
                        self.create_strip_split(split);
                    }
                } else if let Some(pane) = pane {
                    self.add_tab = Some((pane, split.map(str::to_owned)));
                } else {
                    self.create(split);
                }
                ui.close();
            }
        }
        let tabs_in_pane = pane.and_then(|pane| {
            if strip {
                self.strip_pane_tabs.get(&pane).cloned()
            } else {
                self.pane_tabs.get(&pane).cloned()
            }
        });
        if let Some(tabs) = tabs_in_pane.filter(|tabs| tabs.len() > 1) {
            ui.separator();
            ui.weak("Tabs in this pane");
            for tab in tabs {
                let label = match &tab {
                    Tab::Terminal(sid) => self
                        .state
                        .sessions
                        .iter()
                        .find(|s| &s.id == sid)
                        .map(|s| s.label.clone())
                        .unwrap_or_else(|| "Terminal".into()),
                    Tab::Diff { path, .. } | Tab::Image { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    Tab::NativeEditor { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    Tab::Browser { target, .. } => target.title(),
                    Tab::Player => "Player".into(),
                    Tab::CommitLog { .. } => "Commit Log".into(),
                    Tab::Blame { path, .. } => format!(
                        "Blame {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                };
                if appearance::menu_item(ui, &label, "Terminal", "").clicked() {
                    if strip {
                        self.focus_strip_tab = Some(tab);
                    } else {
                        self.focus_tab = Some(tab);
                    }
                    ui.close();
                }
            }
        }
    }
    pub(super) fn rename_action(&mut self, ui: &mut egui::Ui, sid: &str, surface: RenameSurface) {
        let shortcut = self.shortcut_label("rename_terminal");
        if appearance::menu_item(ui, "Rename terminal…", "Pencil", &shortcut).clicked() {
            self.begin_rename(sid, surface);
            ui.close();
        }
    }
    pub(super) fn inline_rename(
        &mut self,
        ui: &mut egui::Ui,
        sid: &str,
        surface: RenameSurface,
        rect: egui::Rect,
    ) {
        if !self.renaming(sid, surface) {
            return;
        }
        let Some(title) = self.rename_session.as_ref().map(|(_, title)| title.clone()) else {
            return;
        };
        let mut title = title;
        let starting = self.rename_focus;
        let response = ui.put(
            rect,
            egui::TextEdit::singleline(&mut title)
                .id_salt(("inline-terminal-title", sid, surface))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(rect.width()),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "rename-input", response.rect);
        self.rename_painted = true;
        self.rename_field_id = Some(response.id);
        if starting {
            response.request_focus();
            self.rename_focus = false;
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(title.chars().count()),
                    )));
                state.store(ui.ctx(), response.id);
            }
        }
        let valid = !title.trim().is_empty() && title.trim().len() <= 256;
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        // The field owns this frame's keyboard input, including when Enter or
        // blur commits it before a terminal widget is rendered later in the frame.
        ui.input_mut(|input| {
            input.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Key { .. }
                )
            });
        });
        if escape || (!starting && response.lost_focus() && !valid) {
            self.rename_session = None;
            response.surrender_focus();
        } else if valid && (enter || (!starting && response.lost_focus())) {
            self.send(Request::Rename {
                session: sid.into(),
                label: title.trim().into(),
            });
            self.rename_session = None;
            response.surrender_focus();
        } else {
            self.rename_session = Some((sid.into(), title));
            if enter {
                response.request_focus();
            }
            response.on_hover_text(if valid {
                "Enter to save · Esc to cancel"
            } else {
                "Enter a non-empty, shorter title"
            });
        }
    }
    fn image_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_images.insert(path.into());
        let mut as_text = false;
        let mut reload = false;
        let mut fit = false;
        let mut actual = false;
        appearance::wrapping_path_row(ui, &services::compact_path(path), |ui| {
            reload = ui.button("Reload").clicked();
            as_text = ui.button("Open as text").clicked();
            if ui.button("Open externally").clicked() {
                let _ = self.jobs.send(Job::External(path.into()));
            }
            if let Some(texture) = self
                .images
                .get(path)
                .and_then(|preview| preview.texture.as_ref())
            {
                ui.weak(format!(
                    "{} × {} pixels",
                    texture.size()[0],
                    texture.size()[1]
                ));
                let fit_btn = ui.button("Fit");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "image-fit", fit_btn.rect);
                fit = fit_btn.clicked();
                actual = ui.button("100%").clicked();
            }
        });
        if as_text {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if reload && let Some(preview) = self.images.get_mut(path) {
            if let Some(cancel) = preview.cancellation.take() {
                cancel.cancel();
            }
            preview.loading = false;
            preview.error = None;
        }
        if !self.images.contains_key(path) && self.images.len() >= 8 {
            ui.weak("Close another image preview to load this image.");
            return;
        }
        let preview = self.images.entry(path.into()).or_default();
        if reload || (!preview.loading && preview.texture.is_none() && preview.error.is_none()) {
            self.image_generation = self.image_generation.wrapping_add(1);
            preview.generation = self.image_generation;
            preview.cancellation = self
                .image_jobs
                .try_send((path.into(), preview.generation))
                .ok();
            preview.loading = preview.cancellation.is_some();
            if !preview.loading {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        if fit {
            preview.scene = egui::Rect::NOTHING;
        }
        if actual && let Some(size) = preview.texture.as_ref().map(egui::TextureHandle::size_vec2) {
            let center = size.to_pos2();
            preview.scene = egui::Rect::from_center_size(
                egui::pos2(center.x * 0.5, center.y * 0.5),
                ui.available_size(),
            );
        }
        preview.show(ui);
    }
    fn browser_view(&mut self, ui: &mut egui::Ui, key: String, target: &BrowserTarget) {
        let href = crate::browser::href(target).unwrap_or_else(|_| target.title());
        let mut draft = self
            .browser_urls
            .remove(&key)
            .unwrap_or_else(|| href.clone());
        if draft.is_empty() {
            draft = href;
        }
        let mut as_text = false;
        let mut system = false;
        let mut reload = false;
        let mut back = false;
        let mut forward = false;
        let mut go = false;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("Back").clicked();
            forward = ui.button("Forward").clicked();
            let url = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .desired_width(240.0)
                    .hint_text("https://"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-url", url.rect);
            let go_btn = ui.button("Go");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-go", go_btn.rect);
            go = go_btn.clicked()
                || (url.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            reload = ui.button("Reload").clicked();
            as_text = target.file().is_some() && ui.button("Open as text").clicked();
            let open = ui.button("Open in browser");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "html-open-browser", open.rect);
            system = open.clicked();
        });
        if go
            && let Ok(next) = BrowserTarget::from_http_url(&draft)
            && next != *target
        {
            self.browser_submit = Some((key.clone(), next));
        }
        self.browser_urls.insert(key.clone(), draft);
        if back {
            self.browser_host.go_back(&key);
        }
        if forward {
            self.browser_host.go_forward(&key);
        }
        if reload && !self.browser_host.reload_view(&key) {
            self.browser_host.drop_view(&key);
        }
        if as_text && let Some(path) = target.file() {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if system {
            match target {
                BrowserTarget::File(path) => self.open_in_browser(path),
                BrowserTarget::Url(url) => {
                    let _ = self.jobs.send(Job::Browser(url.clone()));
                }
            }
        }
        if let Some(error) = self.browser_host.error(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.weak("Open in browser to view the page in your system browser.");
        }
        let rect = ui.available_rect_before_wrap();
        let _ = ui.allocate_rect(rect, egui::Sense::hover());
        self.visible_browsers
            .push(crate::browser_host::VisibleBrowser {
                key,
                target: target.clone(),
                rect,
            });
    }
    fn diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Diff {
            cwd,
            path,
            staged: _,
        } = tab
        else {
            return;
        };
        let is_md = crate::markdown::supported(path);
        let key = tab.key();
        let split = self.diff_split.contains(&key);
        ui.horizontal(|ui| {
            ui.strong(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
            if let Some(letter) = self.context.as_ref().and_then(|context| {
                context
                    .decorations
                    .get(&cwd.join(path))
                    .copied()
                    .filter(|letter| *letter != ' ')
            }) {
                ui.colored_label(
                    sidebar_ui::git_color(&self.theme, letter),
                    letter.to_string(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let more = appearance::menu_button(ui, "…", |ui| {
                    if appearance::menu_item(ui, "Unified", "Rows2", "").clicked() {
                        self.diff_split.remove(&key);
                        ui.close();
                    }
                    ui.separator();
                    let ignore = self.diff_ignore_ws.contains(&key);
                    if appearance::menu_item(
                        ui,
                        if ignore {
                            "Show whitespace"
                        } else {
                            "Ignore whitespace"
                        },
                        "Eraser",
                        "",
                    )
                    .clicked()
                    {
                        if ignore {
                            self.diff_ignore_ws.remove(&key);
                        } else {
                            self.diff_ignore_ws.insert(key.clone());
                        }
                        self.diff_preview.remove(&key);
                        self.loading.insert(key.clone());
                        self.diffs.remove(&key);
                        let _ = self.jobs.send(Job::Diff(tab.clone()));
                        ui.close();
                    }
                    if is_md {
                        let preview = self.diff_preview.contains(&key);
                        if appearance::menu_item(
                            ui,
                            if preview { "Hide preview" } else { "Preview" },
                            "FileText",
                            "",
                        )
                        .clicked()
                        {
                            if preview {
                                self.diff_preview.remove(&key);
                            } else {
                                self.diff_preview.insert(key.clone());
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if appearance::menu_item(ui, "Refresh", "RefreshCw", "").clicked() {
                        self.diff_preview.remove(&key);
                        self.diff_ignore_ws.remove(&key);
                        self.loading.insert(key.clone());
                        let _ = self.jobs.send(Job::Diff(tab.clone()));
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Diff view");
                let _ = more;
                let side_by_side = ui.selectable_label(split, "Side by side");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-side-by-side", side_by_side.rect);
                if side_by_side.clicked() {
                    if split {
                        self.diff_split.remove(&key);
                    } else {
                        self.diff_split.insert(key.clone());
                    }
                }
            });
        });
        if !self.diffs.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::Diff(tab.clone()));
        }
        match self.diffs.get(&key) {
            Some(Ok(doc)) => {
                if is_md && self.diff_preview.contains(&key) {
                    let path = cwd.join(path);
                    if let Some(link) =
                        paint_markdown_diff_preview(ui, doc, &mut self.markdown, &path, &key)
                    {
                        match link {
                            markdown::Link::File(path) => self.open_file(path, None, None, false),
                            markdown::Link::Web(url) => {
                                let _ = self.jobs.send(Job::Browser(url));
                            }
                        }
                    }
                } else {
                    let split = self.diff_split.contains(&key);
                    let colors = DiffColors {
                        added: appearance::color(&self.theme.git_added),
                        deleted: appearance::color(&self.theme.git_deleted),
                        accent: appearance::color(&self.theme.accent),
                        text: appearance::color(&self.theme.text),
                    };
                    let doc = std::sync::Arc::clone(doc);
                    let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
                    paint_diff_document(
                        ui,
                        &doc,
                        split,
                        colors,
                        &key,
                        &mut self.diff_split_ratio,
                        &mut sync,
                    );
                    self.diff_split_scroll.insert(key.clone(), sync);
                }
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
            None => {
                ui.spinner();
            }
        }
    }

    fn commit_log_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::CommitLog { cwd } = tab else { return };
        let key = tab.key();
        let log = self.git_logs.entry(key.clone()).or_insert_with(|| {
            match git_log::fetch_log(cwd.as_path()) {
                Ok(log) => log,
                Err(e) => git_log::CommitLog {
                    error: Some(e),
                    ..Default::default()
                },
            }
        });
        if let Some(error) = &log.error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            return;
        }
        ui.columns(3, |cols| {
            // Left: branch tree
            let Some(col) = cols.get_mut(0) else {
                return;
            };
            col.strong("Branches");
            col.separator();
            appearance::sidebar_scroll("branches").show(col, |ui| {
                for rf in &log.refs {
                    let icon = match rf.kind {
                        git_log::RefKind::LocalBranch => "\u{2398}",
                        git_log::RefKind::RemoteBranch => "\u{1F310}",
                        git_log::RefKind::Tag => "\u{2B50}",
                        git_log::RefKind::Head => "\u{1F4CC}",
                    };
                    let selected = rf.name == log.active_branch;
                    let text = if selected {
                        format!("{} {} \u{2713}", icon, rf.name)
                    } else {
                        format!("{} {}", icon, rf.name)
                    };
                    let _ = ui.selectable_label(selected, &text);
                }
            });
            // Center: commit list
            let Some(col) = cols.get_mut(1) else {
                return;
            };
            col.strong("Commits");
            col.separator();
            let selected_hash = self.selected_commit.clone();
            appearance::sidebar_scroll("commits").show(col, |ui| {
                for commit in &log.commits {
                    let selected = selected_hash.as_deref() == Some(&commit.hash);
                    let response = ui.horizontal(|ui| {
                        if selected {
                            ui.colored_label(appearance::color(&self.theme.accent), "\u{25CF}");
                        } else {
                            ui.add(egui::Label::new(
                                egui::RichText::new("\u{25CF}")
                                    .size(10.0)
                                    .color(appearance::color(&self.theme.text)),
                            ));
                        }
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&commit.subject));
                            ui.horizontal(|ui| {
                                ui.weak(&commit.short_hash);
                                ui.weak(&commit.date);
                                ui.weak(&commit.author);
                            });
                        });
                    });
                    if response.response.clicked() {
                        self.selected_commit = Some(commit.hash.clone());
                    }
                }
            });
            // Right: commit details
            let Some(col) = cols.get_mut(2) else {
                return;
            };
            col.strong("Details");
            col.separator();
            if let Some(hash) = &self.selected_commit {
                if let Some(commit) = log.commits.iter().find(|c| &c.hash == hash) {
                    col.horizontal(|ui| {
                        ui.weak("Hash: ");
                        ui.label(&commit.short_hash);
                    });
                    col.horizontal(|ui| {
                        ui.weak("Author: ");
                        ui.label(&commit.author);
                    });
                    col.horizontal(|ui| {
                        ui.weak("Date: ");
                        ui.label(&commit.date);
                    });
                    col.separator();
                    col.label(&commit.message);
                }
            } else if let Some(first) = log.commits.first() {
                self.selected_commit = Some(first.hash.clone());
            }
        });
    }
    fn blame_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Blame { cwd, path } = tab else {
            return;
        };
        ui.weak(format!("Blame: {}", path.display()));
        ui.separator();
        match git_log::fetch_blame(cwd, path) {
            Ok(data) => {
                appearance::sidebar_scroll("blame").show(ui, |ui| {
                    for entry in &data.entries {
                        ui.horizontal(|ui| {
                            let shown = entry.hash.len().min(7);
                            ui.weak(entry.hash.get(..shown).unwrap_or(entry.hash.as_str()));
                            ui.weak(&entry.author);
                            ui.weak(&entry.date);
                            ui.weak("| ");
                            ui.monospace(&entry.code);
                        });
                    }
                });
            }
            Err(e) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), e);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct DiffColors {
    added: Color32,
    deleted: Color32,
    accent: Color32,
    text: Color32,
}

#[derive(Clone, Copy)]
enum DiffGutter {
    Unified,
    Old,
    New,
}

struct DiffMetrics {
    font: egui::FontId,
    digit_w: f32,
    row_h: f32,
    digits: u32,
}

impl DiffMetrics {
    fn measure(ui: &egui::Ui, digits: u32) -> Self {
        let Some(font) = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Monospace)
            .cloned()
        else {
            return Self {
                font: egui::FontId::monospace(12.0),
                digit_w: 0.0,
                row_h: 0.0,
                digits,
            };
        };
        let (digit_w, row_h) = ui
            .ctx()
            .fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
        Self {
            font,
            digit_w,
            row_h,
            digits,
        }
    }

    fn number_w(&self) -> f32 {
        // Gutter digit column is a UI coordinate.
        #[allow(clippy::cast_precision_loss)]
        let digits = self.digits as f32;
        self.digit_w * digits
    }
}

#[derive(Clone, Copy)]
struct GutterCols {
    old_right: Option<f32>,
    new_right: Option<f32>,
    sign: f32,
    code: f32,
}

struct DiffPaint<'a> {
    width: f32,
    line: &'a diff::DiffLine,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffSidePaint<'a> {
    size: egui::Vec2,
    line: Option<&'a diff::DiffLine>,
    gutter: DiffGutter,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffGutterPaint<'a> {
    rect: egui::Rect,
    cols: GutterCols,
    line: &'a diff::DiffLine,
    sign: &'static str,
    colors: DiffColors,
    metrics: &'a DiffMetrics,
}

struct DiffNumberPaint<'a> {
    top: f32,
    right: f32,
    number: Option<u32>,
    metrics: &'a DiffMetrics,
    color: Color32,
}

// Widths are document-wide so vertical virtualization cannot shrink the horizontal
// scroll range when the longest line leaves the viewport.
#[derive(Clone)]
struct DiffWidths {
    fingerprint: egui::Id,
    content: f32,
}

fn diff_content_width(
    ui: &egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    metrics: &DiffMetrics,
    scroll_key: &str,
) -> f32 {
    let cache_id = egui::Id::new(("diff-width", scroll_key, split));
    let fingerprint = egui::Id::new((
        doc,
        &metrics.font,
        metrics.digit_w.to_bits(),
        metrics.row_h.to_bits(),
        ui.ctx().pixels_per_point().to_bits(),
    ));
    if let Some(cached) = ui
        .ctx()
        .data_mut(|data| data.get_temp::<DiffWidths>(cache_id))
        && cached.fingerprint == fingerprint
    {
        return cached.content;
    }
    let lines: Box<dyn Iterator<Item = &diff::DiffLine>> = if split {
        Box::new(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        Box::new(doc.unified.iter())
    };
    let gutter = if split {
        DiffGutter::Old
    } else {
        DiffGutter::Unified
    };
    let code_width = lines
        .map(|line| {
            let text = line
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>();
            ui.painter()
                .layout_no_wrap(text, metrics.font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let content = gutter_cols(metrics, gutter).code + code_width;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            cache_id,
            DiffWidths {
                fingerprint,
                content,
            },
        )
    });
    content
}

fn paint_diff_document(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    split: bool,
    colors: DiffColors,
    scroll_key: &str,
    ratio: &mut f32,
    sync: &mut f32,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let rows = diff_row_count(doc, split);
    let digits = diff_doc_digits(doc, split);
    let metrics = DiffMetrics::measure(ui, digits);
    let content = diff_content_width(ui, doc, split, &metrics, scroll_key);
    if split {
        paint_diff_split(
            ui, doc, colors, &metrics, content, rows, scroll_key, ratio, sync,
        )
    } else {
        let width = content.max(ui.available_width());
        let output = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::both()
                    .id_salt((scroll_key, false))
                    .auto_shrink([false, false])
                    .show_rows(ui, metrics.row_h, rows, |ui, range| {
                        ui.set_min_width(width);
                        for index in range {
                            let Some(line) = doc.unified.get(index) else {
                                continue;
                            };
                            paint_diff_line(
                                ui,
                                DiffPaint {
                                    width,
                                    line,
                                    gutter: DiffGutter::Unified,
                                    colors,
                                    metrics: &metrics,
                                },
                            );
                        }
                    })
            })
            .inner;
        vec![output]
    }
}

const DIFF_SPLIT_GAP: f32 = 6.0;
const DIFF_SPLIT_MIN_PANE: f32 = 40.0;

#[derive(Clone, Copy)]
enum SplitPane {
    Left,
    Right,
}

// Each side owns its horizontal scroll so the divider can resize them without
// clipping long lines; the vertical offset is mirrored so both sides stay in step.
#[allow(clippy::too_many_arguments)]
fn paint_diff_split(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    colors: DiffColors,
    metrics: &DiffMetrics,
    content: f32,
    rows: usize,
    scroll_key: &str,
    ratio: &mut f32,
    sync: &mut f32,
) -> Vec<egui::scroll_area::ScrollAreaOutput<()>> {
    let full = ui.available_rect_before_wrap();
    let usable = (full.width() - DIFF_SPLIT_GAP).max(1.0);
    let min_pane = DIFF_SPLIT_MIN_PANE.min(usable / 2.0);
    let left_width = (*ratio * usable).clamp(min_pane, usable - min_pane);
    let left_rect = egui::Rect::from_min_size(full.min, egui::vec2(left_width, full.height()));
    let gap_rect = egui::Rect::from_min_size(
        egui::pos2(full.left() + left_width, full.top()),
        egui::vec2(DIFF_SPLIT_GAP, full.height()),
    );
    let right_rect = egui::Rect::from_min_max(egui::pos2(gap_rect.right(), full.top()), full.max);
    // Paint both panes at the same frame-start offset so corresponding lines
    // stay aligned; whichever pane the user scrolled becomes next frame's shared
    // offset.
    let frame_sync = *sync;
    let left = paint_diff_pane(
        ui,
        left_rect,
        doc,
        SplitPane::Left,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        frame_sync,
    );
    let right = paint_diff_pane(
        ui,
        right_rect,
        doc,
        SplitPane::Right,
        colors,
        metrics,
        content,
        rows,
        scroll_key,
        frame_sync,
    );
    // Adopt the pane the pointer is over so sub-pixel input accumulates there
    // and an inactive pane's clamp cannot override the active pane. With the
    // pointer elsewhere (keyboard, divider drag), fall back to whichever pane
    // moved further from the frame-start offset.
    let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
    let over_left = pointer.is_some_and(|pos| left_rect.contains(pos));
    let over_right = pointer.is_some_and(|pos| right_rect.contains(pos));
    let left_offset = left.state.offset.y;
    let right_offset = right.state.offset.y;
    *sync = match (over_left, over_right) {
        (true, false) => left_offset,
        (false, true) => right_offset,
        _ => {
            if (left_offset - frame_sync).abs() >= (right_offset - frame_sync).abs() {
                left_offset
            } else {
                right_offset
            }
        }
    };
    ui.allocate_rect(full, egui::Sense::hover());
    let response = ui.interact(
        gap_rect,
        ui.id().with(("diff-split-drag", scroll_key)),
        egui::Sense::drag(),
    );
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        *ratio = ((pos.x - full.left()) / usable).clamp(min_pane / usable, 1.0 - min_pane / usable);
    }
    let stroke = if response.hovered() || response.dragged() {
        egui::Stroke::new(1.5, colors.accent)
    } else {
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    ui.painter()
        .vline(gap_rect.center().x, full.y_range(), stroke);
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "diff-split-handle", gap_rect);
    vec![left, right]
}

#[allow(clippy::too_many_arguments)]
fn paint_diff_pane(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &diff::DiffDocument,
    pane: SplitPane,
    colors: DiffColors,
    metrics: &DiffMetrics,
    content: f32,
    rows: usize,
    scroll_key: &str,
    sync: f32,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let side = u8::from(matches!(pane, SplitPane::Left));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            egui::ScrollArea::both()
                .id_salt((scroll_key, "split", side))
                .auto_shrink([false, false])
                .vertical_scroll_offset(sync)
                .show_rows(ui, metrics.row_h, rows, |ui, range| {
                    ui.set_min_width(content);
                    for index in range {
                        paint_diff_side(
                            ui,
                            DiffSidePaint {
                                size: egui::vec2(content, metrics.row_h),
                                line: doc.split.get(index).and_then(|row| match pane {
                                    SplitPane::Left => row.left.as_ref(),
                                    SplitPane::Right => row.right.as_ref(),
                                }),
                                gutter: match pane {
                                    SplitPane::Left => DiffGutter::Old,
                                    SplitPane::Right => DiffGutter::New,
                                },
                                colors,
                                metrics,
                            },
                        );
                    }
                })
        },
    )
    .inner
}
fn paint_markdown_diff_preview(
    ui: &mut egui::Ui,
    doc: &diff::DiffDocument,
    previews: &mut markdown::Previews,
    path: &std::path::Path,
    scroll_key: &str,
) -> Option<markdown::Link> {
    let mut link = None;
    ui.columns(2, |columns| {
        for (index, (col, (text, label))) in columns
            .iter_mut()
            .zip([
                (&doc.left_text, &doc.left_label),
                (&doc.right_text, &doc.right_label),
            ])
            .enumerate()
        {
            col.vertical(|ui| {
                ui.strong(label.as_str());
                let key = format!("diff-preview:{scroll_key}:{index}");
                if let Some(clicked) = previews.snapshot(&key, path, text).show(ui, &key) {
                    link = Some(clicked);
                }
            });
        }
    });
    link
}
fn diff_row_count(doc: &diff::DiffDocument, split: bool) -> usize {
    if split {
        doc.split.len()
    } else {
        doc.unified.len()
    }
}

fn diff_doc_digits(doc: &diff::DiffDocument, split: bool) -> u32 {
    if split {
        diff_gutter_digits(
            doc.split
                .iter()
                .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
                .flatten(),
        )
    } else {
        diff_gutter_digits(doc.unified.iter())
    }
}

fn diff_gutter_digits<'a>(lines: impl IntoIterator<Item = &'a diff::DiffLine>) -> u32 {
    let max = lines
        .into_iter()
        .flat_map(|line| [line.old_no, line.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    max.ilog10().saturating_add(1).max(4)
}

fn gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let number_w = metrics.number_w();
    let mut x = pad;
    match gutter {
        DiffGutter::Unified => {
            let old_right = x + number_w;
            x = old_right + pad;
            let new_right = x + number_w;
            x = new_right + pad;
            let sign = x;
            GutterCols {
                old_right: Some(old_right),
                new_right: Some(new_right),
                sign,
                code: sign + metrics.digit_w + pad,
            }
        }
        DiffGutter::Old | DiffGutter::New => side_gutter_cols(metrics, gutter),
    }
}

fn side_gutter_cols(metrics: &DiffMetrics, gutter: DiffGutter) -> GutterCols {
    let pad = metrics.digit_w;
    let right = pad + metrics.number_w();
    let sign = right + pad;
    let code = sign + metrics.digit_w + pad;
    GutterCols {
        old_right: matches!(gutter, DiffGutter::Old).then_some(right),
        new_right: matches!(gutter, DiffGutter::New).then_some(right),
        sign,
        code,
    }
}

fn paint_diff_side(ui: &mut egui::Ui, paint: DiffSidePaint<'_>) {
    let DiffSidePaint {
        size,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    ui.allocate_ui(size, |ui| {
        ui.set_min_size(size);
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
        if let Some(line) = line {
            paint_diff_line(
                ui,
                DiffPaint {
                    width: size.x,
                    line,
                    gutter,
                    colors,
                    metrics,
                },
            );
        }
    });
}

fn diff_row_style(kind: diff::LineKind, colors: DiffColors) -> (Color32, &'static str) {
    match kind {
        diff::LineKind::Insert => (tint(colors.added, 40), "+"),
        diff::LineKind::Delete => (tint(colors.deleted, 40), "-"),
        diff::LineKind::Hunk => (tint(colors.accent, 24), " "),
        diff::LineKind::Equal => (Color32::TRANSPARENT, " "),
    }
}

fn diff_code_job(
    line: &diff::DiffLine,
    font: &egui::FontId,
    colors: DiffColors,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping::no_max_width(),
        break_on_newline: false,
        ..Default::default()
    };
    for span in &line.spans {
        let intra = match (line.kind, span.intra) {
            (diff::LineKind::Insert, diff::Intra::Change) => tint(colors.added, 90),
            (diff::LineKind::Delete, diff::Intra::Change) => tint(colors.deleted, 90),
            _ => Color32::TRANSPARENT,
        };
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: if line.kind == diff::LineKind::Hunk {
                    colors.accent
                } else {
                    Color32::from_rgb(span.rgb[0], span.rgb[1], span.rgb[2])
                },
                background: intra,
                ..Default::default()
            },
        );
    }
    if job.text.is_empty() {
        job.append(
            " ",
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: colors.text,
                ..Default::default()
            },
        );
    }
    job
}

fn fill_diff_row(ui: &egui::Ui, rect: egui::Rect, gutter_w: f32, bg: Color32) {
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let gutter =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
    ui.painter()
        .rect_filled(gutter, 0.0, Color32::from_black_alpha(40));
}

fn paint_diff_number(ui: &egui::Ui, paint: DiffNumberPaint<'_>) {
    let DiffNumberPaint {
        top,
        right,
        number,
        metrics,
        color,
    } = paint;
    let Some(number) = number else {
        return;
    };
    let galley = ui
        .painter()
        .layout_no_wrap(number.to_string(), metrics.font.clone(), color);
    ui.painter().galley(
        egui::pos2(right - galley.size().x, top).round(),
        galley,
        color,
    );
}

fn paint_diff_gutter(ui: &egui::Ui, paint: DiffGutterPaint<'_>) {
    let DiffGutterPaint {
        rect,
        cols,
        line,
        sign,
        colors,
        metrics,
    } = paint;
    if line.kind == diff::LineKind::Hunk {
        return;
    }
    let weak = ui.visuals().weak_text_color();
    if let Some(right) = cols.old_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.old_no,
                metrics,
                color: weak,
            },
        );
    }
    if let Some(right) = cols.new_right {
        paint_diff_number(
            ui,
            DiffNumberPaint {
                top: rect.top(),
                right: rect.left() + right,
                number: line.new_no,
                metrics,
                color: weak,
            },
        );
    }
    if sign != " " {
        let color = match line.kind {
            diff::LineKind::Insert => colors.added,
            diff::LineKind::Delete => colors.deleted,
            diff::LineKind::Hunk | diff::LineKind::Equal => weak,
        };
        let galley = ui
            .painter()
            .layout_no_wrap(sign.to_owned(), metrics.font.clone(), color);
        ui.painter().galley(
            egui::pos2(rect.left() + cols.sign, rect.top()).round(),
            galley,
            color,
        );
    }
}

fn paint_diff_line(ui: &mut egui::Ui, paint: DiffPaint<'_>) {
    let DiffPaint {
        width,
        line,
        gutter,
        colors,
        metrics,
    } = paint;
    let (bg, sign) = diff_row_style(line.kind, colors);
    let cols = gutter_cols(metrics, gutter);
    let code = ui
        .painter()
        .layout_job(diff_code_job(line, &metrics.font, colors));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, metrics.row_h), egui::Sense::hover());
    fill_diff_row(ui, rect, cols.code, bg);
    paint_diff_gutter(
        ui,
        DiffGutterPaint {
            rect,
            cols,
            line,
            sign,
            colors,
            metrics,
        },
    );
    ui.painter().galley(
        egui::pos2(rect.left() + cols.code, rect.top()).round(),
        code,
        colors.text,
    );
}

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub(super) struct Viewer<'a> {
    pub(super) app: &'a mut App,
    /// True when rendering the IDE strip dock instead of the main dock.
    /// Creation queues, pane maps, and focus targets switch docks; the
    /// strip shows native tab bars (the main dock titles panes with
    /// captions because its tabs live in the workspace strip).
    pub(super) strip: bool,
}
impl TabViewer for Viewer<'_> {
    fn on_add(&mut self, path: egui_dock::NodePath) {
        if self.strip {
            self.app.add_strip_tab = Some((path, None));
        } else {
            self.app.add_tab = Some((path, None));
        }
    }
    type Tab = Tab;
    fn show_tab_bar(&self, _path: egui_dock::NodePath) -> bool {
        self.strip
    }
    fn trailing_controls_width(&self) -> f32 {
        let actions = if self.strip {
            appearance::strip_terminal_actions_width() + 4.0
        } else {
            0.0
        };
        actions + 28.0
    }
    fn trailing_controls(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        // The dock's own style can set a taller interact size; pin it so every
        // toolbar control resolves to the same square and centers share a row.
        ui.spacing_mut().interact_size =
            egui::vec2(appearance::TERMINAL_BUTTON, appearance::TERMINAL_BUTTON);
        ui.spacing_mut().item_spacing.x = 4.0;
        if self.strip {
            self.strip_terminal_actions(ui, path);
        }
        #[cfg(feature = "test-support")]
        {
            let rect = ui.max_rect();

            diagnostics::record(
                ui.ctx(),
                "pane-plus",
                rect.translate(egui::vec2(-24.0, 0.0)),
            );
        }
        let response = appearance::icon_menu_button(ui, "ChevronDown", |ui| {
            for (label, direction) in [
                ("New tab", None),
                ("Split up", Some("up")),
                ("Split down", Some("down")),
                ("Split left", Some("left")),
                ("Split right", Some("right")),
            ] {
                let response = appearance::menu_item(
                    ui,
                    label,
                    match direction {
                        Some("up") => "PanelTopClose",
                        Some("down") => "PanelBottomClose",
                        Some("left") => "PanelLeftClose",
                        Some("right") => "PanelRightClose",
                        _ => "Plus",
                    },
                    &self.app.shortcut_label(split_action(direction)),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), label, response.rect);
                if response.clicked() {
                    if self.strip {
                        self.app.add_strip_tab = Some((path, direction.map(str::to_owned)));
                    } else {
                        self.app.add_tab = Some((path, direction.map(str::to_owned)));
                    }
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("New tab or split this pane");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-dropdown", response.rect);
        let _ = response;
    }
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(tab.key())
    }
    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Image { path } => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
            Tab::Browser { target, .. } => target.title().into(),
            Tab::Player => "Player".into(),
            Tab::Terminal(sid) => {
                let label = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .map(|s| s.label.clone())
                    .unwrap_or("Session".into());
                // Split-pane headers and terminal-strip tabs share the
                // presentation model: waiting attention first, then failures.
                let presented = self.app.present_session(sid);
                if presented.attention.waiting() > 0 {
                    format!("● {label}").into()
                } else if presented.attention.failed > 0 {
                    format!("▲ {label}").into()
                } else {
                    label.into()
                }
            }
            Tab::Diff { path, staged, .. } => format!(
                "{} {}",
                if *staged { "Staged:" } else { "Diff:" },
                path.file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
            Tab::NativeEditor { path } => format!(
                "{}{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                if self.app.native_dirty(path) {
                    " ●"
                } else {
                    ""
                }
            )
            .into(),
            Tab::CommitLog { .. } => "Commit Log".into(),
            Tab::Blame { path, .. } => format!(
                "Blame {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
        }
    }
    fn tab_leading_width(&self, tab: &Tab) -> f32 {
        match tab {
            Tab::Terminal(sid) => self.app.tab_leading(sid).width(),
            _ => 0.0,
        }
    }
    fn paint_tab_leading(&mut self, ui: &mut egui::Ui, rect: egui::Rect, tab: &mut Tab) {
        let Tab::Terminal(sid) = tab else {
            return;
        };
        let leading = self.app.tab_leading(sid);
        let mut cursor = rect.left();
        let mut paint = |icon: &str, tint: egui::Color32, spin: bool| {
            appearance::paint_status_icon(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(
                        cursor + appearance::TERMINAL_LEADING_SLOT / 2.0,
                        rect.center().y,
                    ),
                    egui::vec2(13.0, 13.0),
                ),
                icon,
                tint,
                spin,
            );
            cursor += appearance::TERMINAL_LEADING_SLOT;
        };
        if let Some(brand) = leading.brand {
            paint(brand, appearance::ICON_COLOR, false);
        }
        if let Some((icon, tint, spin)) = leading.status {
            paint(icon, tint, spin);
        } else if let Some(kind) = leading.kind {
            paint(kind, appearance::ICON_COLOR, false);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("strip-tab-icon:{sid}"), rect);
    }
    fn allowed_in_windows(&self, _: &mut Tab) -> bool {
        false
    }
    fn scroll_bars(&self, _: &Tab) -> [bool; 2] {
        [false, false]
    }
    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        if let Tab::Terminal(sid) = tab {
            if self
                .app
                .state
                .sessions
                .iter()
                .any(|s| s.id == *sid && s.lifecycle.live())
            {
                self.app.close_session = Some(sid.clone());
                return OnCloseResponse::Ignore;
            }
            self.app.backends.remove(sid);
        }
        if let Tab::NativeEditor { path } = tab {
            if self.app.native_dirty(path) {
                self.app.native_close_prompt = Some(path.clone());
                return OnCloseResponse::Ignore;
            }
            self.app.native_docs.remove(path);
        }
        OnCloseResponse::Close
    }
    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        if let Tab::Terminal(sid) = tab {
            let presented = self.app.present_session(sid);
            response.clone().on_hover_ui(|ui| {
                ui.label(presented.diagnostics(now()));
            });
        }
        if response.hovered() && !response.dragged() {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.double_clicked()
            && let Tab::Terminal(sid) = tab
        {
            self.app.begin_rename(sid, RenameSurface::Pane);
        }
        if response.clicked()
            && let Tab::Terminal(sid) = tab
        {
            self.app.active_session = Some(sid.clone());
            self.app.send(Request::Focus {
                session: sid.clone(),
            });
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Tab, pane: egui_dock::NodePath) {
        if let Tab::Terminal(sid) = tab {
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            ui.separator();
        }
        self.app.new_terminal_menu(ui, Some(pane), self.strip);
        ui.separator();
        if let Tab::Terminal(sid) = tab {
            if appearance::menu_item(
                ui,
                "Search scrollback",
                "Search",
                &self.app.shortcut_label("search_scrollback"),
            )
            .clicked()
            {
                self.app.open_scrollback_search(ui.ctx(), sid);
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "Clear saved scrollback",
                "Eraser",
                &self.app.shortcut_label("clear_scrollback"),
            )
            .clicked()
            {
                self.app.send(Request::ClearHistory {
                    session: Some(sid.clone()),
                });
                self.app.texts.remove(&format!("history:{sid}"));
                ui.close();
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Image { path } => self.app.image_view(ui, path),
            Tab::Browser { id, target } => {
                let key = id.clone();
                self.app.browser_view(ui, key, target);
            }
            Tab::Player => {
                self.app.open_player();
                ui.close();
            }
            Tab::Diff { .. } => self.app.diff_view(ui, tab),
            Tab::NativeEditor { path } => self.app.native_editor_view(ui, path),
            Tab::CommitLog { .. } => self.app.commit_log_view(ui, tab),
            Tab::Blame { .. } => self.app.blame_view(ui, tab),
            Tab::Terminal(sid) => {
                let Some(session) = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == *sid)
                    .cloned()
                else {
                    // The daemon pruned the record (a plain shell or editor
                    // without an agent resume handle). Drop the stale tab
                    // instead of leaving a dead placeholder behind.
                    self.app.queue_unavailable_tab_close(sid);
                    return;
                };
                let editing = self.app.renaming(sid, RenameSurface::Pane);
                if self.strip {
                    // Native tab bars title strip panes, so there is no
                    // caption chrome (close, drag, and menus all live on the
                    // tab). Only an in-progress rename needs a row of its own.
                    if editing {
                        let slot = ui.allocate_response(
                            egui::vec2(ui.available_width(), 26.0),
                            egui::Sense::hover(),
                        );
                        self.app
                            .inline_rename(ui, sid, RenameSurface::Pane, slot.rect);
                    }
                } else {
                    let pane = self
                        .app
                        .pane_by_tab
                        .get(&Tab::Terminal(sid.clone()).key())
                        .copied();
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let is_markdown = markdown::available(&session);
                    // A lone pane's name is already the workspace tab. Keep the
                    // drag and close row, but don't paint the title again.
                    let lone = self
                        .app
                        .pane_index
                        .as_ref()
                        .is_some_and(|index| index.tabs == 1);
                    let branch = self.app.branch_at(&session.cwd);
                    let git_tip = match &branch {
                        Some(name) => format!("{name}\nOpen Git"),
                        None => "Open Git".into(),
                    };
                    let vertical_tip = self.app.action_tip("Split vertically", "split_right");
                    let horizontal_tip = self.app.action_tip("Split horizontally", "split_down");
                    let (response, close, actions) = if is_markdown {
                        let header = self.markdown_header(ui, &session, editing);
                        (header.0, header.1, None)
                    } else {
                        let leading = self.app.tab_leading(sid);
                        let bar = appearance::terminal_bar(
                            ui,
                            appearance::TerminalBarSpec {
                                title: if editing || lone { "" } else { &session.label },
                                active: self.app.active_session.as_ref() == Some(sid),
                                branch: branch.as_deref(),
                                status: self.app.terminal_status_color(&session),
                                git_tip: &git_tip,
                                vertical_tip: &vertical_tip,
                                horizontal_tip: &horizontal_tip,
                                brand: leading.brand,
                                status_icon: leading.status.map(|(icon, _, _)| icon),
                                status_tint: leading.status.map(|(_, tint, _)| tint),
                                spin: leading.status.is_some_and(|(_, _, spin)| spin),
                                kind: if leading.status.is_none() {
                                    leading.kind
                                } else {
                                    None
                                },
                            },
                        );
                        (
                            bar.bar,
                            Some(bar.close),
                            Some((bar.git, bar.split_vertical, bar.split_horizontal)),
                        )
                    };
                    #[cfg(feature = "test-support")]
                    if !is_markdown && !lone && !editing {
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-caption:{}", session.label),
                            response.rect,
                        );
                    }
                    let controls_left = actions
                        .as_ref()
                        .map(|(git, _, _)| git.rect.left() - 4.0)
                        .unwrap_or_else(|| {
                            response.rect.right()
                                - if close.is_some() && !is_markdown {
                                    28.0
                                } else {
                                    8.0
                                }
                        });
                    if editing {
                        self.app.inline_rename(
                            ui,
                            sid,
                            RenameSurface::Pane,
                            egui::Rect::from_min_max(
                                egui::pos2(response.rect.min.x + 8.0, response.rect.min.y + 1.0),
                                egui::pos2(controls_left, response.rect.bottom() - 1.0),
                            ),
                        );
                    }
                    let closing = close.as_ref().is_some_and(eframe::egui::Response::clicked);
                    let git_clicked = actions.as_ref().is_some_and(|(git, _, _)| git.clicked());
                    let split_vertical = actions
                        .as_ref()
                        .is_some_and(|(_, split, _)| split.clicked());
                    let split_horizontal = actions
                        .as_ref()
                        .is_some_and(|(_, _, split)| split.clicked());
                    let on_control = close.as_ref().is_some_and(eframe::egui::Response::hovered)
                        || actions.as_ref().is_some_and(|(git, vertical, horizontal)| {
                            git.hovered() || vertical.hovered() || horizontal.hovered()
                        });
                    #[cfg(feature = "test-support")]
                    if let Some(close) = &close {
                        diagnostics::record(ui.ctx(), &format!("editor-close:{sid}"), close.rect);
                        diagnostics::record(ui.ctx(), &format!("pane-close:{sid}"), close.rect);
                    }
                    #[cfg(feature = "test-support")]
                    if let Some((git, vertical, horizontal)) = &actions {
                        diagnostics::record(ui.ctx(), &format!("pane-git:{sid}"), git.rect);
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-split-vertical:{sid}"),
                            vertical.rect,
                        );
                        diagnostics::record(
                            ui.ctx(),
                            &format!("pane-split-horizontal:{sid}"),
                            horizontal.rect,
                        );
                    }
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("pane-drag:{sid}"), response.rect);
                    // Caption drag starts a pane move. The drop lands on another
                    // split leaf (rearrange) or a workspace strip tab (move
                    // across top-level tabs); clicks still focus as before.
                    if response.drag_started() && !editing && !closing && !on_control {
                        self.app.pane_drag = Some(Tab::Terminal(sid.clone()));
                        // Snapshot once: the ghost reuses it every frame instead
                        // of re-reading the live grid while it scrolls.
                        self.app.pane_drag_snapshot = self
                            .app
                            .backends
                            .get(sid)
                            .map(snapshot_rows)
                            .unwrap_or_default();
                    }
                    if self.app.pane_drag.as_ref() == Some(&Tab::Terminal(sid.clone()))
                        && response.hovered()
                        && ui.input(|i| i.pointer.any_down())
                    {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                    }
                    if closing {
                        self.app.close_session = Some(sid.clone());
                    }
                    if git_clicked {
                        self.app.active_session = Some(sid.clone());
                        self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                        self.app.show_git_sidebar();
                    }
                    if split_vertical || split_horizontal {
                        self.app.active_session = Some(sid.clone());
                        self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                    }
                    if let Some(pane) = pane {
                        if split_vertical {
                            self.app.add_tab = Some((pane, Some("right".into())));
                        }
                        if split_horizontal {
                            self.app.add_tab = Some((pane, Some("down".into())));
                        }
                    }
                    if response.clicked() && !closing && !editing && !git_clicked && !on_control {
                        self.app.active_session = Some(sid.clone());
                        self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                    }
                    if response.double_clicked() && !closing && !editing && !on_control {
                        self.app.begin_rename(sid, RenameSurface::Pane);
                    }
                    appearance::context_menu(&response, |ui| {
                        if let Some(pane) = pane {
                            self.context_menu(ui, &mut Tab::Terminal(sid.clone()), pane);
                        }
                    });
                }
                if !session.lifecycle.live() {
                    self.app.backends.remove(sid);
                    ui.colored_label(
                        appearance::color(&self.app.theme.status_waiting),
                        format!(
                            "{:?} session — commands will not be run automatically",
                            session.lifecycle
                        ),
                    );
                    for a in self
                        .app
                        .state
                        .agents
                        .iter()
                        .filter(|a| a.session_id == *sid)
                    {
                        if let Some(resume) = &a.resume {
                            let text = resume.display();
                            ui.horizontal(|ui| {
                                ui.monospace(&text);
                                if ui.small_button("Copy resume").clicked() {
                                    ui.ctx().copy_text(text);
                                }
                            });
                        } else {
                            ui.weak(format!("{}: resume command unavailable", a.kind));
                        }
                    }
                    if ui.button("Open a fresh shell here").clicked() {
                        let _ = self.app.jobs.send(Job::rpc(
                            Request::Create {
                                project: session.project_id.clone(),
                                cwd: Some(session.cwd.clone()),
                                file: None,
                                line: None,
                                column: None,
                                editor: false,
                            },
                            if self.strip {
                                After::Strip
                            } else {
                                After::Create(None)
                            },
                        ));
                    }
                    if session.truncated {
                        ui.weak("Some saved output was pruned or unavailable.");
                    }
                    let key = format!("history:{sid}");
                    if !self.app.texts.contains_key(&key) && self.app.loading.insert(key.clone()) {
                        let _ = self.app.jobs.send(Job::rpc(
                            Request::History {
                                session: sid.clone(),
                            },
                            After::Text(key.clone()),
                        ));
                    }
                    let sid_key = sid.clone();
                    let query = self
                        .app
                        .history_filter
                        .entry(sid_key.clone())
                        .or_default()
                        .clone();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(
                                self.app.history_filter.entry(sid_key.clone()).or_default(),
                            )
                            .id(egui::Id::new(("history-filter", sid_key)))
                            .hint_text("Filter saved scrollback")
                            .desired_width(220.0),
                        );
                        if ui.small_button("✕").on_hover_text("Back").clicked() {
                            self.app.search_session = None;
                            self.app.history_filter.remove(sid);
                        }
                    });
                    if let Some(text) = self.app.texts.get(&key) {
                        let lines = filter_history_lines(text, &query);
                        if !query.is_empty() {
                            ui.monospace(format!("{} matching lines", lines.len()));
                        }
                        egui::ScrollArea::both().id_salt(key).show_rows(
                            ui,
                            18.0,
                            lines.len(),
                            |ui, range| {
                                for row in range {
                                    if let Some(line) = lines.get(row) {
                                        ui.monospace(*line);
                                    }
                                }
                            },
                        );
                    }
                    return;
                }
                if !self.app.connected {
                    ui.weak("Reconnecting to session daemon…");
                    return;
                }
                self.app.draw_unsaved_close_bar(ui, sid);
                if markdown::available(&session) {
                    self.markdown_view(ui, &session);
                } else {
                    self.terminal_view(ui, &session);
                }
            }
        }
    }
}

impl App {
    fn draw_unsaved_close_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some((target, ids, error)) = self.unsaved_close_prompt(sid) else {
            return;
        };
        let enabled = !self.editor_close_busy(&ids);
        let bar = appearance::unsaved_close_bar(
            ui,
            appearance::UnsavedCloseBar {
                theme: &self.theme,
                message: &error,
                enabled,
            },
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "Save and close", bar.save.rect);
            diagnostics::record(ui.ctx(), "Discard changes", bar.discard.rect);
            diagnostics::record(ui.ctx(), "Cancel", bar.cancel.rect);
        }
        if let Some(choice) = bar.choice() {
            self.apply_unsaved_close_choice(choice, target, ids);
        }
    }
}

impl Viewer<'_> {
    fn strip_terminal_actions(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        let tabs = self
            .app
            .strip_pane_tabs
            .get(&path)
            .cloned()
            .unwrap_or_default();
        let sid = self
            .app
            .active_session
            .as_ref()
            .filter(|sid| {
                tabs.iter()
                    .any(|tab| matches!(tab, Tab::Terminal(id) if id == *sid))
            })
            .cloned()
            .or_else(|| {
                tabs.iter().find_map(|tab| match tab {
                    Tab::Terminal(id) => Some(id.clone()),
                    _ => None,
                })
            });
        let branch = sid.as_ref().and_then(|sid| {
            self.app
                .state
                .sessions
                .iter()
                .find(|session| &session.id == sid)
                .and_then(|session| self.app.branch_at(&session.cwd))
        });
        let git_tip = match &branch {
            Some(name) => format!("{name}\nOpen Git"),
            None => "Open Git".into(),
        };
        let vertical_tip = self.app.action_tip("Split vertically", "split_right");
        let horizontal_tip = self.app.action_tip("Split horizontally", "split_down");
        let (dot, _) = ui.allocate_exact_size(egui::vec2(14.0, 22.0), egui::Sense::hover());
        if let Some(color) = sid.as_ref().and_then(|sid| {
            self.app
                .state
                .sessions
                .iter()
                .find(|session| &session.id == sid)
                .and_then(|session| self.app.terminal_status_color(session))
        }) {
            ui.painter().circle_filled(dot.center(), 3.5, color);
        }
        let git_icon = appearance::sidebar_action(ui, "GitBranch", &git_tip);
        let branch_label = ui
            .add_sized(
                [appearance::TERMINAL_BRANCH_MAX, appearance::TERMINAL_BUTTON],
                egui::Label::new(
                    egui::RichText::new(branch.as_deref().unwrap_or(""))
                        .size(12.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate()
                .sense(egui::Sense::click()),
            )
            .on_hover_text(&git_tip);
        let git = git_icon.union(branch_label);
        let split_vertical = appearance::sidebar_action(ui, "Columns2", &vertical_tip);
        let split_horizontal = appearance::sidebar_action(ui, "Rows2", &horizontal_tip);
        #[cfg(feature = "test-support")]
        if let Some(sid) = &sid {
            diagnostics::record(ui.ctx(), &format!("pane-git:{sid}"), git.rect);
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-vertical:{sid}"),
                split_vertical.rect,
            );
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-horizontal:{sid}"),
                split_horizontal.rect,
            );
        }
        if git.clicked() {
            if let Some(sid) = sid.clone() {
                self.app.active_session = Some(sid.clone());
                self.app.focus_strip_tab = Some(Tab::Terminal(sid));
            }
            self.app.show_git_sidebar();
        }
        if split_vertical.clicked() || split_horizontal.clicked() {
            if let Some(sid) = &sid {
                self.app.active_session = Some(sid.clone());
                self.app.focus_strip_tab = Some(Tab::Terminal(sid.clone()));
            }
            let direction = if split_vertical.clicked() {
                "right"
            } else {
                "down"
            };
            self.app.add_strip_tab = Some((path, Some(direction.into())));
        }
    }

    fn markdown_header(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        editing: bool,
    ) -> (egui::Response, Option<egui::Response>) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let header = appearance::markdown_header(
            ui,
            &session.label,
            self.app.active_session.as_ref() == Some(sid),
            editing,
            mode,
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "markdown-title", header.title.rect);
            diagnostics::record(ui.ctx(), "markdown-refresh", header.refresh.rect);
        }
        for (option, response) in header.modes {
            #[cfg(feature = "test-support")]
            {
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{}", option.label()),
                    response.rect,
                );
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{sid}:{}", option.label()),
                    response.rect,
                );
            }
            if response.clicked() {
                self.app.markdown.retain(sid).editor_focused = option != markdown::Mode::Preview;
                self.app.active_session = Some(sid.clone());
                self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                if option == markdown::Mode::default() {
                    self.app.preferences.markdown_modes.remove(sid);
                } else {
                    self.app
                        .preferences
                        .markdown_modes
                        .insert(sid.clone(), option);
                }
            }
        }
        if header.refresh.clicked() {
            self.app.markdown.refresh(ui.ctx());
        }
        (header.title, Some(header.close))
    }
    fn markdown_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let preview = self.app.markdown.retain(sid);
        if mode == markdown::Mode::Edit {
            preview.editor_focused = true;
        } else {
            preview.pointer_focus(ui);
        }
        let mut link = None;
        if mode != markdown::Mode::Edit {
            self.app.markdown.watch(markdown::Source::new(
                &self.app.state.session_paths(&self.app.paths, &session.id),
                session,
            ));
        }
        match mode {
            markdown::Mode::Edit => self.terminal_view(ui, session),
            markdown::Mode::Preview => link = self.app.markdown.retain(sid).show(ui, sid),
            markdown::Mode::Split => {
                let width = ui.available_width();
                egui::Panel::left(egui::Id::new(("markdown-editor", sid)))
                    .resizable(true)
                    .default_size(width * 0.5)
                    .size_range(80.0..=(width - 80.0).max(80.0))
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.terminal_view(ui, session));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| link = self.app.markdown.retain(sid).show(ui, sid));
            }
        }
        if let Some(link) = link {
            match link {
                markdown::Link::File(path) => self.app.terminal_action(
                    ui.ctx(),
                    session,
                    &services::Target::File(path, None, None),
                    FileAction::Open,
                ),
                markdown::Link::Web(url) => {
                    let _ = self.app.jobs.send(Job::Browser(url));
                }
            }
        }
    }
    fn find_paint_for(&self, sid: &str) -> Option<egui_term::FindPaint> {
        let find = self.app.terminal_find.get(sid)?;
        if find.query.is_empty() || find.outcome.matches.is_empty() {
            return None;
        }
        Some(egui_term::FindPaint {
            matches: find.outcome.matches.clone(),
            current: find.current,
        })
    }

    /// In-terminal find bar. Search runs GUI-side over the live grid plus
    /// retained scrollback and never writes to the PTY.
    fn terminal_find_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        if !self.app.terminal_find.contains_key(sid) {
            return;
        }
        let mut close = false;
        let mut reveal: Option<i32> = None;
        {
            let Some(find) = self.app.terminal_find.get_mut(sid) else {
                return;
            };
            let Some(backend) = self.app.backends.get_mut(sid) else {
                return;
            };
            let id = egui::Id::new(("terminal-find", sid));
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut find.query)
                        .id(id)
                        .hint_text("Find in terminal")
                        .desired_width(220.0),
                );
                if response.changed() {
                    find.current = 0;
                }
                let case_label = if find.case_insensitive { "aa" } else { "Aa" };
                if ui
                    .small_button(case_label)
                    .on_hover_text("Match case")
                    .clicked()
                {
                    find.case_insensitive = !find.case_insensitive;
                    find.current = 0;
                }
                let now = Instant::now();
                let stale = find
                    .last_search
                    .is_none_or(|t| now.duration_since(t).as_millis() > 250);
                if find.dirty() || (stale && !find.query.is_empty()) {
                    let was_dirty = find.dirty();
                    find.outcome = backend.find(&find.query, find.case_insensitive);
                    find.searched_query.clone_from(&find.query);
                    find.searched_case = find.case_insensitive;
                    find.last_search = Some(now);
                    if was_dirty {
                        find.current = 0;
                    } else {
                        find.current = find
                            .current
                            .min(find.outcome.matches.len().saturating_sub(1));
                    }
                    if let Some(hit) = find.outcome.matches.get(find.current) {
                        reveal = Some(hit.line);
                    }
                }
                let total = find.outcome.matches.len();
                let label = if find.query.is_empty() {
                    String::new()
                } else if total == 0 {
                    "No matches".to_string()
                } else {
                    let mut text = format!("{}/{}", find.current.saturating_add(1), total);
                    if find.outcome.truncated {
                        text.push('+');
                    }
                    text
                };
                ui.monospace(label);
                let shift = ui.input(|i| i.modifiers.shift);
                if ui
                    .small_button("↑")
                    .on_hover_text("Previous (Shift+Enter)")
                    .clicked()
                    || (response.has_focus()
                        && shift
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    find.step(-1);
                    reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
                }
                if ui.small_button("↓").on_hover_text("Next (Enter)").clicked()
                    || (response.has_focus()
                        && !shift
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    find.step(1);
                    reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
                }
                if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
                if response.has_focus()
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    close = true;
                }
                if response.has_focus() {
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
            });
        }
        if let Some(line) = reveal
            && let Some(backend) = self.app.backends.get_mut(sid)
        {
            backend.reveal_grid_line(line);
        }
        if close {
            self.app.terminal_find.remove(sid);
        }
    }

    pub(super) fn terminal_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        self.app.visible_sessions.insert(sid.clone());
        if !self.app.backends.contains_key(sid)
            && self
                .app
                .attach_budget
                .get(sid)
                .is_some_and(|budget| budget.exhausted(&session.cwd, 0))
        {
            let message = self.app.attach_error.get(sid).cloned().unwrap_or_else(|| {
                "Stopped retrying this terminal after repeated attachment failures. Check the session service, then retry."
                    .into()
            });
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
                if ui.small_button("Retry").clicked() {
                    self.app.attach_budget.remove(sid);
                    self.app.attach_error.remove(sid);
                    self.app.attach_started.remove(sid);
                }
            });
            return;
        }
        if !self.app.backends.contains_key(sid) {
            let id = self.app.next_backend;
            self.app.next_backend = self.app.next_backend.saturating_add(1);
            let owner = self
                .app
                .state
                .generations
                .iter()
                .find(|g| g.owner.id == session.generation);
            let endpoint = owner
                .map(|g| g.owner.paths())
                .unwrap_or_else(|| self.app.paths.clone());
            let helper_result = owner
                .and_then(|g| g.helper.clone())
                .map(Ok)
                .unwrap_or_else(|| installation::attachment_helper(&self.app.state));
            let helper = match helper_result {
                Ok(p) => p,
                Err(e) => {
                    ui.label(e.to_string());
                    return;
                }
            };
            let font = egui_term::TerminalFont::new(egui_term::FontSettings {
                font_type: egui::FontId::monospace(self.app.state.settings.font_size),
            });
            let size = egui_term::TerminalSize::from_pane(
                ui.available_size(),
                font.font_measure(ui.ctx()),
            );
            match TerminalBackend::new(
                id,
                ui.ctx().clone(),
                self.app.pty_tx.clone(),
                egui_term::BackendSettings {
                    shell: helper.to_string_lossy().into(),
                    args: vec![
                        "attach".into(),
                        sid.clone(),
                        endpoint.data.to_string_lossy().into(),
                        endpoint.runtime.to_string_lossy().into(),
                    ],
                    working_directory: None,
                    size,
                },
            ) {
                Ok(b) => {
                    self.app.attach_started.insert(sid.clone(), Instant::now());
                    self.app.backends.insert(sid.clone(), b);
                    self.app.backend_ids.insert(id, sid.clone());
                }
                Err(e) => {
                    let message = format!("Cannot attach terminal: {e}");
                    self.app
                        .attach_budget
                        .entry(sid.clone())
                        .or_default()
                        .record(&session.cwd, 0, true);
                    self.app.attach_error.insert(sid.clone(), message.clone());
                    ui.colored_label(appearance::color(&self.app.theme.status_failed), message);
                    return;
                }
            }
        }
        self.terminal_find_bar(ui, sid);
        let input_enabled = if self.strip {
            self.app.strip_terminal_input_enabled(sid)
        } else {
            self.app.terminal_input_enabled(sid)
        };
        let find_open = self.app.terminal_find.contains_key(sid);
        let focused = input_enabled
            && !find_open
            && self.app.active_session.as_ref() == Some(sid)
            && self
                .app
                .markdown
                .entries
                .get(sid)
                .is_none_or(|p| p.editor_focused);
        if focused {
            let count = ui
                .input_mut(|input| clipboard::take_image_paste(&mut input.events, input.modifiers));
            for _ in 0..count {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
            }
        }
        let find_paint = self.find_paint_for(sid);
        let Some(backend) = self.app.backends.get_mut(sid) else {
            return;
        };
        backend.set_painted(true);
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(self.app.state.settings.font_size),
        });
        let theme = cached_terminal_theme(&mut self.app.terminal_theme, &self.app.theme);
        let view = TerminalView::new(ui, backend)
            .external_links(true)
            .set_theme(theme)
            .set_focus(focused)
            .set_font(font)
            .set_size(ui.available_size())
            .find_highlight(find_paint);
        let response = ui.add_enabled(input_enabled, view);
        #[cfg(feature = "test-support")]
        {
            if session.kind == SessionKind::Shell {
                diagnostics::record(ui.ctx(), "terminal", response.rect);
            }
            diagnostics::record(ui.ctx(), &format!("terminal:{sid}"), response.rect);
            if session.kind == SessionKind::Editor {
                diagnostics::record(ui.ctx(), "editor-terminal", response.rect);
            }
        }
        if response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            self.app.terminal_pressed(ui.ctx(), sid);
        }
        let dragging = ui.input(|i| i.pointer.any_down());
        let (mouse_reporting, target) = {
            let Some(backend) = self.app.backends.get(sid) else {
                return;
            };
            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                let content = backend.last_content();
                let snapshot = (
                    focused,
                    content.display_offset,
                    content.terminal_mode.bits(),
                );
                let key = egui::Id::new(("scroll-evidence", sid));
                if ui.ctx().data(|d| d.get_temp::<(bool, usize, u32)>(key)) != Some(snapshot) {
                    eprintln!(
                        "Scroll evidence: session={sid} focused={} offset={} modes={}",
                        snapshot.0, snapshot.1, snapshot.2
                    );
                    ui.ctx().data_mut(|d| d.insert_temp(key, snapshot));
                }
            }
            let mouse_reporting = backend
                .last_content()
                .terminal_mode
                .intersects(egui_term::TerminalMode::MOUSE_MODE);
            let target = if dragging {
                None
            } else {
                response.hover_pos().and_then(|pos| {
                    backend.target_at(pos.x - response.rect.left(), pos.y - response.rect.top())
                })
            };
            (mouse_reporting, target)
        };
        let token = target.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), token);
        if !token.is_empty()
            && !self.app.targets.contains_key(&key)
            && self.app.loading.insert(key.clone())
        {
            let _ = self.app.jobs.send(Job::ResolveTarget(
                key.clone(),
                token.clone(),
                session.cwd.clone(),
            ));
        }
        let resolved = self.app.targets.get(&key).cloned().flatten();
        if let (Some(target), Some(resolved)) = (&target, &resolved) {
            if !ui.input(|i| i.pointer.any_down()) && !mouse_reporting {
                for rect in &target.rects {
                    let rect = rect.translate(response.rect.min.to_vec2());
                    ui.painter().with_clip_rect(response.rect).line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        egui::Stroke::new(1.0, appearance::color(&self.app.theme.accent)),
                    );
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let popup_key = self.app.hover_popup.as_ref().map(|popup| popup.key.clone());
                if should_replace_hover_popup(
                    &mut self.app.hover,
                    &key,
                    popup_key.as_deref(),
                    HOVER_POPUP_DELAY,
                ) {
                    let rect = target
                        .rects
                        .first()
                        .copied()
                        .unwrap_or(egui::Rect::ZERO)
                        .translate(response.rect.min.to_vec2());
                    self.app.hover_popup = Some(HoverPopup {
                        session: sid.clone(),
                        key: key.clone(),
                        target: resolved.clone(),
                        rect,
                    });
                }
                if let Some(wake) = hover_popup_wake(
                    &self.app.hover,
                    &key,
                    self.app
                        .hover_popup
                        .as_ref()
                        .map(|popup| popup.key.as_str()),
                    HOVER_POPUP_DELAY,
                ) {
                    ui.ctx().request_repaint_after(wake);
                }
            }
        } else if response.contains_pointer() {
            self.app.hover = None;
        }
        if !mouse_reporting
            && response.clicked()
            && ui.input(|i| {
                if cfg!(target_os = "macos") {
                    i.modifiers.mac_cmd
                } else {
                    i.modifiers.ctrl
                }
            })
        {
            if let Some(target) = &resolved {
                self.app
                    .terminal_action(ui.ctx(), session, target, FileAction::Open);
            } else if !token.is_empty() {
                self.app.pending_target_action = Some((key.clone(), session.clone()));
            }
        }
        let menu_key = egui::Id::new(("terminal-menu-target", sid.as_str()));
        if response.secondary_clicked() {
            let selected = self
                .app
                .backends
                .get(sid)
                .map_or_else(String::new, TerminalBackend::selectable_content);
            let text = if selected.trim().is_empty() {
                token.clone()
            } else {
                selected
            };
            let key = format!("target:{}:{}:{}", sid, session.cwd.display(), text);
            ui.ctx().data_mut(|d| d.insert_temp(menu_key, key.clone()));
            if !self.app.targets.contains_key(&key) && self.app.loading.insert(key.clone()) {
                let _ = self
                    .app
                    .jobs
                    .send(Job::ResolveTarget(key, text, session.cwd.clone()));
            }
        }
        appearance::context_menu(&response, |ui| {
            let cwd = session.cwd.display().to_string();
            appearance::target_header(ui, &cwd, &cwd);
            let selected = self
                .app
                .backends
                .get(sid)
                .map_or_else(String::new, TerminalBackend::selectable_content);
            let command = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+Shift+"
            };
            ui.add_enabled_ui(!selected.is_empty(), |ui| {
                if appearance::menu_item(ui, "Copy", "Copy", &format!("{command}C")).clicked() {
                    ui.ctx().copy_text(selected.clone());
                    ui.close();
                }
            });
            if appearance::menu_item(
                ui,
                "Select all",
                "TextSelect",
                &self.app.shortcut_label("select_all"),
            )
            .clicked()
            {
                if let Some(backend) = self.app.backends.get_mut(sid) {
                    backend.select_all();
                }
                ui.close();
            }
            if appearance::menu_item(ui, "Paste", "Clipboard", &format!("{command}V")).clicked() {
                let _ = self.app.jobs.send(Job::PasteClipboard(sid.clone()));
                ui.close();
            }
            ui.separator();
            let key = Tab::Terminal(sid.clone()).key();
            let pane = if self.strip {
                self.app.strip_pane_by_tab.get(&key).copied()
            } else {
                self.app.pane_by_tab.get(&key).copied()
            };
            self.app.new_terminal_menu(ui, pane, self.strip);
            ui.separator();
            let key = ui
                .ctx()
                .data(|d| d.get_temp::<String>(menu_key))
                .unwrap_or_default();
            if let Some(Some(target)) = self.app.targets.get(&key).cloned() {
                appearance::target_header(ui, &target.compact(), &target.display());
                if let Some(action) = file_actions::menu(ui, file_actions::target_menu(&target)) {
                    self.app.terminal_action(ui.ctx(), session, &target, action);
                }
            }
            if appearance::menu_item(
                ui,
                "Open file path…",
                "File",
                &self.app.shortcut_label("open_file"),
            )
            .clicked()
            {
                self.app.path_text.clone_from(&selected);
                self.app.open_path = true;
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(
                ui,
                "Search scrollback",
                "Search",
                &self.app.shortcut_label("search_scrollback"),
            )
            .clicked()
            {
                self.app.open_scrollback_search(ui.ctx(), sid);
                ui.close();
            }
            if session.kind == SessionKind::Editor && !session.review {
                if appearance::menu_item(
                    ui,
                    "Save all",
                    "Save",
                    &self.app.shortcut_label("editor_save"),
                )
                .clicked()
                {
                    self.app.send(Request::EditorSave {
                        session: sid.clone(),
                    });
                    ui.close();
                }
                if appearance::menu_item(
                    ui,
                    "Compare disk",
                    "FileDiff",
                    &self.app.shortcut_label("compare_disk"),
                )
                .clicked()
                {
                    self.app.send(Request::EditorCompare {
                        session: sid.clone(),
                    });
                    ui.close();
                }
            }
            if appearance::menu_item(
                ui,
                "Copy working directory",
                "Folder",
                &self.app.shortcut_label("copy_working_directory"),
            )
            .clicked()
            {
                ui.ctx().copy_text(session.cwd.display().to_string());
                ui.close();
            }
            ui.separator();
            self.app.rename_action(ui, sid, RenameSurface::Pane);
            if appearance::menu_item(
                ui,
                "Close session…",
                "X",
                &self.app.shortcut_label("close_session"),
            )
            .clicked()
            {
                self.app.close_session = Some(sid.clone());
                ui.close();
            }
        });
        if let Some(popup) = self.app.hover_popup.clone()
            && popup.session == *sid
        {
            let mut open = true;
            egui::Popup::from_response(&response)
                .id(egui::Id::new((
                    "terminal-hover",
                    sid.as_str(),
                    popup.key.as_str(),
                )))
                .anchor(popup.rect)
                .open_bool(&mut open)
                .style(appearance::menu_style)
                .show(|ui| {
                    ui.set_max_width(440.0);
                    appearance::target_header(ui, &popup.target.compact(), &popup.target.display());
                    if let Some(action) =
                        file_actions::menu(ui, file_actions::target_menu(&popup.target))
                    {
                        self.app
                            .terminal_action(ui.ctx(), session, &popup.target, action);
                    }
                });
            if !open {
                self.app.hover_popup = None;
                self.app.hover = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ide_mode_icon_differs_from_sidebar_toggles() {
        let ide = header_action_view(HeaderAction::IdeMode).icon;
        assert_ne!(ide, "PanelRight");
        assert_ne!(ide, "PanelLeft");
    }

    #[test]
    fn agents_header_uses_the_bell() {
        let agents = header_action_view(HeaderAction::Tool(SidebarTool::Agents)).icon;
        assert_eq!(agents, "Bell");
    }

    #[test]
    fn terminal_theme_cache_tracks_preview_and_revert() {
        let mut cache = None;
        let original = AppearanceConfig::default();
        let mut preview = original.clone();
        preview.terminal_foreground = "#123456".into();
        preview.terminal_background = "#abcdef".into();
        cached_terminal_theme(&mut cache, &original);
        cached_terminal_theme(&mut cache, &preview);
        let cached = cache.as_ref().unwrap();
        assert_eq!(cached.0, preview.terminal_background);
        assert_eq!(cached.1, preview.terminal_foreground);
        cached_terminal_theme(&mut cache, &original);
        let cached = cache.as_ref().unwrap();
        assert_eq!(cached.0, original.terminal_background);
        assert_eq!(cached.1, original.terminal_foreground);
    }

    use super::*;

    fn span(text: &str) -> diff::DiffSpan {
        diff::DiffSpan {
            text: text.into(),
            rgb: [209, 211, 217],
            intra: diff::Intra::None,
        }
    }

    fn line(
        kind: diff::LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: &str,
    ) -> diff::DiffLine {
        diff::DiffLine {
            kind,
            old_no,
            new_no,
            spans: vec![span(text)],
        }
    }

    fn sample_line() -> diff::DiffLine {
        line(diff::LineKind::Insert, None, Some(1), "visible-diff-marker")
    }

    fn sample_doc() -> diff::DiffDocument {
        diff::DiffDocument {
            left_label: "Index".into(),
            right_label: "Working tree".into(),
            left_text: String::new(),
            right_text: String::new(),
            unified: vec![sample_line()],
            split: vec![],
        }
    }

    fn paint_diff_view(app: &mut App, ctx: &egui::Context, tab: &Tab) -> Vec<(egui::Pos2, String)> {
        let mut painted = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    // egui_dock tab body: no pane scrollbars, expand to the leaf,
                    // then the native viewer owns its own ScrollArea.
                    egui::ScrollArea::new([false, false]).show(ui, |ui| {
                        let available = ui.available_rect_before_wrap();
                        ui.expand_to_include_rect(available);
                        app.diff_view(ui, tab);
                    });
                },
            );
            painted = painted_text(&output.shapes);
            output.textures_delta.clear();
        }
        painted
    }

    fn paint_doc(doc: diff::DiffDocument, split: bool) -> Vec<(egui::Pos2, String)> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        if split {
            app.diff_split.insert(tab.key());
        }
        app.diffs.insert(tab.key(), Ok(doc.into()));
        paint_diff_view(&mut app, &ctx, &tab)
    }

    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Pos2, String)> {
        fn walk(out: &mut Vec<(egui::Pos2, String)>, shape: &egui::Shape) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(out, shape);
                    }
                }
                egui::Shape::Text(text) => out.push((text.pos, text.galley.text().to_owned())),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&mut out, &clipped.shape);
        }
        out
    }

    fn require_text<'a>(
        painted: &'a [(egui::Pos2, String)],
        needle: &str,
    ) -> &'a (egui::Pos2, String) {
        painted
            .iter()
            .find(|(_, text)| text == needle)
            .unwrap_or_else(|| panic!("{needle} was not painted: {painted:?}"))
    }

    #[test]
    fn reopened_diff_uses_unified_after_default_changes() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.selected = Some("project".into());
        let tab = Tab::Diff {
            cwd: dir.path().into(),
            path: dir.path().join("file.md"),
            staged: false,
        };
        let Tab::Diff { cwd, path, staged } = &tab else {
            unreachable!()
        };
        app.context = Some(services::ContextData {
            cwd: cwd.clone(),
            root: Some(cwd.clone()),
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![],
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::default(),
            error: None,
        });
        let open_native = |app: &mut App| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.file_action(
                    ui,
                    if *staged {
                        FileAction::NativeStagedDiff
                    } else {
                        FileAction::NativeWorkingDiff
                    },
                    path,
                    None,
                );
            });
            output.textures_delta.clear();
        };
        app.state.settings.diff_split_default = true;
        open_native(&mut app);
        assert!(app.diff_split.contains(&tab.key()));
        app.layouts.clear();
        app.state.settings.diff_split_default = false;
        open_native(&mut app);
        assert!(!app.diff_split.contains(&tab.key()));
    }

    #[test]
    fn unified_diff_text_stays_in_the_viewport() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, text) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x),
            "code must start at the gutter, not centered or at x=0, got {pos:?} {text}"
        );
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "code galley must not include line numbers, got {text:?}"
        );
    }

    #[test]
    fn diff_view_paints_hunks_in_a_dock_pane() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, _) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x) && (0.0..500.0).contains(&pos.y),
            "hunk text must stay in the pane, got {pos:?}"
        );
    }

    #[test]
    fn equal_lines_share_a_gutter_x() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(8), Some(8), "x"),
                    line(
                        diff::LineKind::Equal,
                        Some(9),
                        Some(9),
                        "this-is-a-much-longer-equal-line",
                    ),
                ],
                split: vec![],
            },
            false,
        );
        let short = require_text(&painted, "x");
        let long = require_text(&painted, "this-is-a-much-longer-equal-line");
        assert!(
            (short.0.x - long.0.x).abs() < 1.0,
            "short and long lines must share a gutter, got {} vs {}",
            short.0.x,
            long.0.x
        );
    }

    #[test]
    fn delete_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(
                    diff::LineKind::Delete,
                    Some(100),
                    None,
                    "removed-line",
                )],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "100");
        let sign = require_text(&painted, "-");
        let code = require_text(&painted, "removed-line");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "minus must sit in its own column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
        assert!(
            code.0.x > sign.0.x,
            "code must start after the sign, got sign={} code={}",
            sign.0.x,
            code.0.x
        );
    }

    #[test]
    fn insert_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Insert, None, Some(102), "added-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "102");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x,
            "plus must sit to the right of the new number, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn split_paints_one_number_per_side() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![],
                split: vec![diff::SplitRow {
                    left: Some(line(diff::LineKind::Delete, Some(5), None, "left-only")),
                    right: Some(line(diff::LineKind::Insert, None, Some(6), "right-only")),
                }],
            },
            true,
        );
        let left = require_text(&painted, "left-only");
        let right = require_text(&painted, "right-only");
        let old_no = require_text(&painted, "5");
        let new_no = require_text(&painted, "6");
        assert!(
            left.0.x < right.0.x,
            "split sides must not stack, got left={} right={}",
            left.0.x,
            right.0.x
        );
        assert!(
            old_no.0.x < left.0.x && old_no.0.x < 200.0,
            "old number must stay on the left gutter, got {old_no:?}"
        );
        assert!(
            new_no.0.x > left.0.x && new_no.0.x < right.0.x,
            "new number must stay on the right gutter, got old={} new={} left={} right={}",
            old_no.0.x,
            new_no.0.x,
            left.0.x,
            right.0.x
        );
    }

    #[test]
    fn diff_rows_use_font_height_not_item_spacing() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(1), Some(1), "row-a"),
                    line(diff::LineKind::Equal, Some(2), Some(2), "row-b"),
                ],
                split: vec![],
            },
            false,
        );
        let a = require_text(&painted, "row-a");
        let b = require_text(&painted, "row-b");
        let pitch = b.0.y - a.0.y;
        assert!(
            (12.0..22.0).contains(&pitch),
            "row pitch must be the font line height, not height+8 item_spacing, got {pitch}"
        );
    }

    #[test]
    fn equal_line_keeps_old_and_new_numbers_apart() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Equal, Some(97), Some(97), "unchanged")],
                split: vec![],
            },
            false,
        );
        let numbers: Vec<_> = painted.iter().filter(|(_, text)| text == "97").collect();
        assert_eq!(
            numbers.len(),
            2,
            "unified equal lines paint old and new numbers separately, got {painted:?}"
        );
        let gap = (numbers[1].0.x - numbers[0].0.x).abs();
        assert!(
            gap > 8.0,
            "old and new 97 must be distinct columns, got {} and {}",
            numbers[0].0.x,
            numbers[1].0.x
        );
    }

    #[test]
    fn hunk_header_paints_no_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Hunk, None, None, "@@ -3,2 +3,2 @@")],
                split: vec![],
            },
            false,
        );
        let header = require_text(&painted, "@@ -3,2 +3,2 @@");
        assert!(
            (40.0..200.0).contains(&header.0.x),
            "hunk text must align with code, got {header:?}"
        );
        assert!(
            painted.iter().all(|(_, text)| text != "3"),
            "hunk rows must not paint fake line numbers, got {painted:?}"
        );
    }

    #[test]
    fn five_digit_line_numbers_still_clear_the_sign() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Insert, None, Some(10000), "wide-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "10000");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "5-digit numbers must not overflow into the sign column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn gutter_digit_columns_grow_with_line_numbers() {
        assert_eq!(diff_gutter_digits(std::iter::empty()), 4);
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(9999),
                Some(9999),
                "n",
            ))),
            4
        );
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(10000),
                Some(10000),
                "n",
            ))),
            5
        );
    }
    #[test]
    fn insertion_only_split_rows_keep_the_right_column() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "old".into(),
                right_label: "new".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![],
                split: vec![
                    diff::SplitRow {
                        left: None,
                        right: Some(line(diff::LineKind::Insert, None, Some(1), "insert-only")),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Equal, Some(1), Some(2), "paired-left")),
                        right: Some(line(
                            diff::LineKind::Equal,
                            Some(1),
                            Some(2),
                            "paired-right",
                        )),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Delete, Some(2), None, "delete-only")),
                        right: None,
                    },
                ],
            },
            true,
        );
        assert_eq!(
            require_text(&painted, "insert-only").0.x,
            require_text(&painted, "paired-right").0.x
        );
        assert_eq!(
            require_text(&painted, "delete-only").0.x,
            require_text(&painted, "paired-left").0.x
        );
    }

    fn scroll_doc(
        ctx: &egui::Context,
        doc: &diff::DiffDocument,
        split: bool,
    ) -> (
        Vec<egui::scroll_area::ScrollAreaOutput<()>>,
        Vec<egui::epaint::ClippedShape>,
    ) {
        let mut scroll = None;
        let mut ratio = 0.5;
        let mut sync = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                scroll = Some(paint_diff_document(
                    ui,
                    doc,
                    split,
                    DiffColors {
                        added: Color32::GREEN,
                        deleted: Color32::RED,
                        accent: Color32::BLUE,
                        text: Color32::WHITE,
                    },
                    "scroll-regression",
                    &mut ratio,
                    &mut sync,
                ));
            },
        );
        output.textures_delta.clear();
        (scroll.unwrap(), output.shapes)
    }

    fn long_doc() -> diff::DiffDocument {
        let unified: Vec<_> = (1..=150)
            .map(|n| {
                line(
                    diff::LineKind::Equal,
                    Some(n),
                    Some(n),
                    &if n == 1 {
                        format!("{}END", "long-line-".repeat(30))
                    } else {
                        format!("short-{n}")
                    },
                )
            })
            .collect();
        let split = unified
            .iter()
            .map(|line| diff::SplitRow {
                left: Some(line.clone()),
                right: Some(line.clone()),
            })
            .collect();
        diff::DiffDocument {
            left_label: "old".into(),
            right_label: "new".into(),
            left_text: String::new(),
            right_text: String::new(),
            unified,
            split,
        }
    }

    #[test]
    fn long_split_lines_have_scroll_space_and_do_not_cross_their_column() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_eq!(areas.len(), 2);
        for area in &areas {
            assert!(area.content_size.x > 1000.0);
            let mut state = area.state;
            state.offset.x = area.content_size.x - area.inner_rect.width();
            state.store(&ctx, area.id);
        }
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let mut long: Vec<_> = shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().ends_with("END")
                {
                    Some((shape.clip_rect, text))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(long.len(), 2);
        long.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
        assert!(long[0].0.right() <= long[1].0.left() + 1.0);
        for (clip, text) in long {
            assert!(
                text.pos.x + text.galley.size().x <= clip.right() + 1.0,
                "split line escaped its pane: {:?} vs {:?}",
                text.pos,
                clip
            );
        }
    }

    #[test]
    fn split_row_pitch_matches_the_virtualized_font_height() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let painted = painted_text(&shapes);
        let a = require_text(&painted, "short-2");
        let b = require_text(&painted, "short-3");
        let font = ctx.global_style().text_styles[&egui::TextStyle::Monospace].clone();
        let height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        assert!((b.0.y - a.0.y - height).abs() <= 1.0);
    }

    #[test]
    fn horizontal_scroll_reaches_the_end_of_the_right_split_line() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        for area in &areas {
            let mut state = area.state;
            state.offset.x = area.content_size.x - area.inner_rect.width();
            state.store(&ctx, area.id);
        }
        let (areas, shapes) = scroll_doc(&ctx, &doc, true);
        let end = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().ends_with("END") => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((end - areas[1].inner_rect.right()).abs() <= 1.0);
    }

    #[test]
    fn split_panes_keep_independent_offsets_and_ids() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        // Unified keeps its own offset while rows virtualize.
        scroll_doc(&ctx, &doc, false);
        let (before, _) = scroll_doc(&ctx, &doc, false);
        let width = before[0].content_size.x;
        let mut state = before[0].state;
        state.offset = egui::vec2(250.0, 900.0);
        state.store(&ctx, before[0].id);
        let (after, _) = scroll_doc(&ctx, &doc, false);
        assert_eq!(after[0].content_size.x, width);
        assert_eq!(after[0].state.offset, egui::vec2(250.0, 900.0));
        // Each split pane scrolls horizontally on its own.
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_ne!(before[0].id, areas[0].id);
        assert_ne!(areas[0].id, areas[1].id);
        let mut left_state = areas[0].state;
        left_state.offset.x = 200.0;
        left_state.store(&ctx, areas[0].id);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_eq!(areas[0].state.offset.x, 200.0);
        assert_ne!(areas[1].state.offset.x, 200.0);
    }

    #[test]
    fn refreshing_a_diff_invalidates_its_cached_width() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let (long, _) = scroll_doc(&ctx, &doc, false);
        let (short, _) = scroll_doc(&ctx, &sample_doc(), false);
        assert!(long[0].content_size.x > short[0].content_size.x * 2.0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn split_divider_drag_rebalances_panes() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            text: Color32::WHITE,
        };
        let mut ratio = 0.5;
        let frame = |events: Vec<egui::Event>, ratio: &mut f32| {
            let mut sync = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    paint_diff_document(ui, &doc, true, colors, "drag-test", ratio, &mut sync);
                },
            );
            output.textures_delta.clear();
        };
        frame(Vec::new(), &mut ratio);
        let handle = ctx
            .data(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", "diff-split-handle")))
            })
            .expect("split handle must be recorded");
        let start = handle.center();
        let dest = start + egui::vec2(150.0, 0.0);
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        frame(vec![egui::Event::PointerMoved(start)], &mut ratio);
        frame(vec![press(start, true)], &mut ratio);
        frame(vec![egui::Event::PointerMoved(dest)], &mut ratio);
        frame(vec![press(dest, false)], &mut ratio);
        assert!(
            ratio > 0.5,
            "divider drag should widen the left pane: {ratio}"
        );
    }

    #[test]
    fn markdown_diff_preview_renders_both_sides() {
        let doc = diff::DiffDocument {
            left_label: "Left".into(),
            right_label: "Right".into(),
            left_text: "# Hello\n\nThis is the old **markdown**.".into(),
            right_text: "# Hello\n\nThis is the *updated* markdown.".into(),
            unified: vec![],
            split: vec![],
        };
        let ctx = egui::Context::default();
        let mut previews = markdown::Previews::new(&ctx);
        for (index, text) in [&doc.left_text, &doc.right_text].into_iter().enumerate() {
            previews.snapshot(
                &format!("diff-preview:test-preview:{index}"),
                std::path::Path::new("/repo/test.md"),
                text,
            );
        }
        previews.wait_prepared();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| {
                paint_markdown_diff_preview(
                    ui,
                    &doc,
                    &mut previews,
                    std::path::Path::new("/repo/test.md"),
                    "test-preview",
                );
            },
        );
        output.textures_delta.clear();
        let painted = painted_text(&output.shapes);
        assert!(painted.iter().any(|(_, t)| t.contains("Hello")));
        assert!(painted.iter().any(|(_, t)| t.contains("Left")));
        assert!(painted.iter().any(|(_, t)| t.contains("Right")));
        assert!(painted.iter().any(|(_, t)| t.contains("updated")));
    }

    #[test]
    fn history_filter_matches_case_insensitively() {
        let text = "cargo build ok\nFAILED to link\nwarning: unused\n";
        assert_eq!(filter_history_lines(text, "").len(), 3);
        assert_eq!(filter_history_lines(text, "failed"), vec!["FAILED to link"]);
        assert_eq!(filter_history_lines(text, "CARGO"), vec!["cargo build ok"]);
        assert!(filter_history_lines(text, "missing").is_empty());
    }

    #[test]
    fn hover_popup_waits_before_the_first_open() {
        let mut hover = None;
        assert!(!should_replace_hover_popup(
            &mut hover,
            "a",
            None,
            HOVER_POPUP_DELAY
        ));
        assert_eq!(hover.as_ref().map(|(key, _)| key.as_str()), Some("a"));
        hover.as_mut().unwrap().1 -= HOVER_POPUP_DELAY;
        assert!(should_replace_hover_popup(
            &mut hover,
            "a",
            None,
            HOVER_POPUP_DELAY
        ));
    }

    #[test]
    fn hover_popup_wake_rests_once_the_popup_is_open() {
        let hover = Some(("a".to_owned(), Instant::now()));
        assert!(hover_popup_wake(&hover, "a", Some("a"), HOVER_POPUP_DELAY).is_none());
        let wake = hover_popup_wake(&hover, "a", None, HOVER_POPUP_DELAY)
            .expect("a fresh hover wakes once");
        assert!(wake > Duration::from_millis(50) && wake <= HOVER_POPUP_DELAY);
    }

    #[test]
    fn hover_popup_switches_when_the_target_changes() {
        let mut hover = Some(("a".into(), Instant::now()));
        assert!(should_replace_hover_popup(
            &mut hover,
            "b",
            Some("a"),
            HOVER_POPUP_DELAY
        ));
        assert_eq!(hover.as_ref().map(|(key, _)| key.as_str()), Some("b"));
        assert!(!should_replace_hover_popup(
            &mut hover,
            "b",
            Some("b"),
            HOVER_POPUP_DELAY
        ));
    }

    #[test]
    fn header_overflow_count_hides_only_what_does_not_fit() {
        let count = HEADER_ACTIONS.len();
        let full = header_row_width(count, false);
        assert_eq!(header_visible_count(full, count), count);
        assert_eq!(header_visible_count(full - 1.0, count), count - 1);
        let three = header_row_width(3, true);
        assert_eq!(header_visible_count(three, count), 3);
        assert_eq!(header_visible_count(three - 1.0, count), 2);
        assert_eq!(header_visible_count(0.0, count), 0);
    }

    fn split_frame(
        ctx: &egui::Context,
        doc: &diff::DiffDocument,
        sync: &mut f32,
        events: Vec<egui::Event>,
    ) -> Vec<(egui::Pos2, String)> {
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            text: Color32::WHITE,
        };
        let mut ratio = 0.5;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                paint_diff_document(ui, doc, true, colors, "sync-regression", &mut ratio, sync);
            },
        );
        output.textures_delta.clear();
        painted_text(&output.shapes)
    }

    fn wheel(delta_y: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, delta_y),
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Every distinct painted text must appear at the same y in both panes, so
    /// corresponding split rows stay aligned.
    fn assert_split_rows_aligned(painted: &[(egui::Pos2, String)]) {
        use std::collections::HashMap;
        let mut ys: HashMap<&str, Vec<f32>> = HashMap::new();
        for (pos, text) in painted {
            ys.entry(text.as_str()).or_default().push(pos.y);
        }
        let mut compared: usize = 0;
        for (text, values) in &ys {
            if values.len() == 2
                && let (Some(left), Some(right)) = (values.first(), values.get(1))
            {
                compared = compared.saturating_add(1);
                assert!(
                    (*left - *right).abs() <= 1.0,
                    "row {text:?} is misaligned across panes: {values:?}"
                );
            }
        }
        assert!(compared > 0, "no corresponding rows were painted");
    }

    #[test]
    fn split_wheel_over_a_pane_moves_both_panes_in_step() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let mut sync = 0.0;
        let over_left = egui::pos2(150.0, 150.0);
        split_frame(&ctx, &doc, &mut sync, vec![]);
        split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        for _ in 0..4 {
            split_frame(
                &ctx,
                &doc,
                &mut sync,
                vec![egui::Event::PointerMoved(over_left), wheel(-60.0)],
            );
        }
        assert!(
            sync > 0.0,
            "wheel over the left pane must move the shared offset: {sync}"
        );
        let painted = split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        assert_split_rows_aligned(&painted);
    }

    #[test]
    fn split_fractional_wheel_input_accumulates() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let mut sync = 0.0;
        let over_left = egui::pos2(150.0, 150.0);
        split_frame(
            &ctx,
            &doc,
            &mut sync,
            vec![egui::Event::PointerMoved(over_left)],
        );
        for _ in 0..8 {
            split_frame(
                &ctx,
                &doc,
                &mut sync,
                vec![egui::Event::PointerMoved(over_left), wheel(-0.2)],
            );
        }
        assert!(
            sync > 0.0,
            "sub-half-point wheel input must accumulate instead of being dropped: {sync}"
        );
    }

    #[test]
    fn closing_a_diff_forgets_its_split_scroll() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: dir.path().into(),
            path: dir.path().join("file.rs"),
            staged: false,
        };
        let key = tab.key();
        app.layouts.insert(
            "project".into(),
            Workspace::from_layout(DockState::new(vec![tab])),
        );
        app.diff_split_scroll.insert(key.clone(), 42.0);
        app.prune_diff_docs();
        assert!(
            app.diff_split_scroll.contains_key(&key),
            "an open diff keeps its cached scroll"
        );
        app.layouts.clear();
        app.prune_diff_docs();
        assert!(
            !app.diff_split_scroll.contains_key(&key),
            "closing the diff tab must forget its cached split scroll"
        );
    }
}
