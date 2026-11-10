//! File tabs in pane tab bars: the file-type icon, and the folder hint
//! after a file name that another tab in the same dock shows. The unsaved
//! dot is the dock's `tab_modified` hook.
use super::super::super::*;
use super::super::file_tabs::hinted_title;
use super::Viewer;

impl Viewer<'_> {
    /// Leaf tabs of the dock under render: the IDE strip, a floating
    /// window, or the main dock. Each map is refreshed before its dock
    /// paints.
    fn dock_tabs(&self) -> impl Iterator<Item = &Tab> {
        let leaves = if self.strip {
            &self.app.strip_pane_tabs
        } else if self.window.is_some() {
            &self.app.float_pane_tabs
        } else {
            &self.app.pane_tabs
        };
        leaves.values().flatten()
    }

    /// Folder hint for `path` when another tab in this dock shows the same
    /// file name.
    fn dock_folder_hint(&self, path: &Path) -> Option<String> {
        self.app.tab_folder_hint(
            path,
            self.dock_tabs().filter_map(|tab| self.app.tab_file(tab)),
        )
    }

    fn weak_text(&self) -> Color32 {
        appearance::color(&self.app.theme.secondary)
    }

    /// Title of a tab named after its file, with the folder hint when
    /// another tab shows the same name.
    pub(super) fn file_title(&self, path: &Path) -> egui::WidgetText {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match self.dock_folder_hint(path) {
            Some(hint) => hinted_title(&name, &hint, Color32::PLACEHOLDER, self.weak_text()).into(),
            None => name.into_owned().into(),
        }
    }

    /// A session tab's `text` in its title color, then the folder hint when
    /// it is an editor named after a file that another tab shows, else the
    /// program's window title.
    pub(super) fn session_title(&self, sid: &str, text: String) -> egui::WidgetText {
        let color = self.app.terminal_title_color(sid);
        let hint = self
            .app
            .named_editor_file(sid)
            .and_then(|file| self.dock_folder_hint(file))
            .or_else(|| self.app.terminal_program_title(sid).map(str::to_owned));
        match hint {
            Some(hint) => hinted_title(&text, &hint, color, self.weak_text()).into(),
            None => egui::RichText::new(text).color(color).into(),
        }
    }

    /// The file-type icon before a file tab's title, sized and placed like
    /// the terminal identity glyphs.
    pub(super) fn paint_file_leading(&self, ui: &egui::Ui, rect: egui::Rect, tab: &Tab) {
        let (Tab::NativeEditor { path } | Tab::Image { path } | Tab::Document { path, .. }) = tab
        else {
            return;
        };
        appearance::paint_status_icon(appearance::PaintStatusIconInput {
            ui,
            rect: egui::Rect::from_center_size(
                egui::pos2(
                    rect.left() + appearance::TERMINAL_LEADING_SLOT / 2.0,
                    rect.center().y,
                ),
                egui::vec2(13.0, 13.0),
            ),
            icon: icons::file_icon(path),
            tint: appearance::icon_color(),
            spin: false,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("file-tab-icon:{}", path.display()), rect);
    }
}
