//! Per-frame sync of the native browser views with the panes of the main
//! window and of the floating windows.
use crate::*;

impl App {
    pub(crate) fn browser_covered(&self) -> bool {
        self.center_covers_workspace()
            || self.command_dialog_open()
            || self.picker.busy()
            || self.close_session.is_some()
            || self.close_workspace.is_some()
            || self.notice_detail_modal_open()
            || self.rename_blocks_input()
    }

    /// Records the popups of the window that `ctx` paints now. Egui
    /// windows and dialogs (middle layer), menus, context menus and combo
    /// boxes (foreground layer), and tooltips would otherwise be drawn
    /// over by the native view. Call once per window, after its panes.
    pub(crate) fn note_browser_covers(&mut self, ctx: &egui::Context) {
        let viewport = ctx.viewport_id();
        if !self
            .visible_browsers
            .iter()
            .any(|pane| pane.viewport == viewport)
        {
            return;
        }
        let covers = &mut self.browser_covers;
        ctx.memory(|memory| {
            covers.extend(
                memory
                    .areas()
                    .visible_layer_ids()
                    .into_iter()
                    .filter(|layer| crate::browser::covers_webview(layer.order))
                    .filter_map(|layer| memory.area_rect(layer.id))
                    .map(|cover| (viewport, cover)),
            );
        });
    }

    /// Runs after [`Self::paint_floating`], so the panes and popups of
    /// every window are known.
    pub(crate) fn sync_browsers(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.note_browser_covers(ctx);
        // A dialog or center page covers all of the main window only.
        if self.browser_covered() {
            self.browser_covers
                .push((egui::ViewportId::ROOT, egui::Rect::EVERYTHING));
        }
        // A view in a closed floating window goes while the window exists.
        let floating = &self.floating;
        self.browser_host
            .retain_windows(|viewport| floating.iter().any(|window| window.viewport == viewport));
        self.browser_host
            .set_devtools(self.preferences.browser.devtools);
        self.browser_host.sync(browser_host::SyncInput {
            ctx,
            frame,
            visible: &self.visible_browsers,
            covers: &self.browser_covers,
            data_dir: &self.paths.data,
        });
        for (key, url) in self.browser_host.take_opens() {
            let project = self
                .open_tabs()
                .find(|(_, tab)| tab.key() == key)
                .map(|(project, _)| project.clone());
            if let Some(project) = project
                && let Err(error) = self.open_browser_url(&project, &url, None)
            {
                self.error.set(format!("Could not open {url}: {error:#}"));
            }
        }
    }
}
