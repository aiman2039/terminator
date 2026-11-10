//! macOS menu-bar status item (Linux tray icon) and the sidebar tools it opens.

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
use crate::chrome::menu_bar;
use crate::*;

impl App {
    /// Bring the window forward from the menu-bar status item.
    #[cfg(any(test, target_os = "macos", target_os = "linux"))]
    pub(crate) fn focus_window_from_menu(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    /// Show the Problems tool in whichever sidebar the current mode
    /// treats as primary (left in IDE mode, right otherwise).
    pub(crate) fn open_problems(&mut self) {
        if self.preferences.ide_mode {
            self.preferences
                .set_left_content(LeftContent::Tool(SidebarTool::Problems));
            self.preferences.left_visible = true;
        } else {
            self.preferences.tool = SidebarTool::Problems;
            self.preferences.visible = true;
        }
    }
    /// Shows the inbox in the right sidebar. Projects there take precedence
    /// over the tool, so clear them or the inbox stays hidden.
    pub(crate) fn open_agents_inbox(&mut self) {
        self.preferences.right_projects = false;
        self.preferences.tool = SidebarTool::Agents;
        self.preferences.visible = true;
        self.preferences.agents_tab = AgentsTab::NeedsAttention;
    }
    /// A status-menu pick: the inbox Go button plus opening the inbox
    /// behind it. A stale id just opens the inbox.
    #[cfg(any(test, target_os = "macos", target_os = "linux"))]
    pub(crate) fn focus_status_notice(&mut self, ctx: &egui::Context, id: &str) {
        Self::focus_window_from_menu(ctx);
        self.open_agents_inbox();
        if let Some(session) = self
            .state
            .notifications
            .iter()
            .find(|notice| notice.id == id)
            .map(|notice| notice.session_id.clone())
        {
            self.go_session(&session);
        }
        self.detail = None;
    }
    /// The status menu items, only when they may differ from the ones last
    /// returned. The cached sidebar index is rebuilt only when notices or
    /// sessions change, so the items are built only then (#211).
    #[cfg(any(test, target_os = "macos", target_os = "linux"))]
    pub(crate) fn changed_status_menu_items(&mut self) -> Option<Vec<updater::StatusMenuItem>> {
        let index = self.sidebar_index();
        if self
            .status_menu_index
            .as_ref()
            .is_some_and(|shown| std::sync::Arc::ptr_eq(shown, &index))
        {
            return None;
        }
        self.status_menu_index = Some(index);
        Some(self.status_menu_items())
    }
    #[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn sync_menu_bar(&mut self, ctx: &egui::Context) {
        if updater::take_status_click() {
            Self::focus_window_from_menu(ctx);
            if self.waiting_notice_count() > 0 {
                self.open_agents_inbox();
            }
        }
        if let Some(items) = self.changed_status_menu_items() {
            updater::sync_status_menu(&items);
        }
        if let Some(id) = updater::take_status_selection() {
            self.focus_status_notice(ctx, &id);
        }
        let waiting = self.waiting_notice_count();
        if self.status_waiting_shown == Some(waiting) {
            return;
        }
        self.status_waiting_shown = Some(waiting);
        let icon = menu_bar::status_icon(waiting);
        updater::sync_status_item(&icon.png, icon.width_pt, icon.height_pt);
        menu_bar::set_dock_badge(waiting);
        updater::sync_status_waiting(waiting);
    }

    #[cfg(not(all(not(test), any(target_os = "macos", target_os = "linux"))))]
    pub(crate) fn sync_menu_bar(&mut self, _ctx: &egui::Context) {}
}
