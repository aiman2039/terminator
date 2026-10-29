//! macOS menu-bar status item and the sidebar tools it opens.

#[cfg(all(not(test), target_os = "macos"))]
use crate::chrome::menu_bar;
use crate::*;

impl App {
    /// Bring the window forward from the menu-bar status item.
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn focus_window_from_menu(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    /// Show the Problems tool in whichever sidebar the current mode
    /// treats as primary (left in IDE mode, right otherwise).
    pub(crate) fn open_problems(&mut self) {
        if self.preferences.ide_mode {
            self.preferences.left_tool = Some(SidebarTool::Problems);
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
    #[cfg(any(test, target_os = "macos"))]
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
    #[cfg(all(not(test), target_os = "macos"))]
    pub(crate) fn sync_menu_bar(&mut self, ctx: &egui::Context) {
        if updater::take_status_click() {
            Self::focus_window_from_menu(ctx);
            if self.waiting_notice_count() > 0 {
                self.open_agents_inbox();
            }
        }
        updater::sync_status_menu(&self.status_menu_items());
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
    }

    #[cfg(not(all(not(test), target_os = "macos")))]
    pub(crate) fn sync_menu_bar(&mut self, _ctx: &egui::Context) {}
}
