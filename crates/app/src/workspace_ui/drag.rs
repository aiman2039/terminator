use super::super::*;
use super::tabs::{TAB_TEXT_MAX, workspace_tab_width};
impl App {
    /// Insertion slot for a strip pointer: how many tab centers sit left
    /// of it. None when the pointer leaves the strip band vertically.
    pub(super) fn strip_insertion_at(
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
    pub(super) fn strip_interior_tab(
        tabs: &[(String, egui::Rect)],
        pos: egui::Pos2,
    ) -> Option<String> {
        const EDGE: f32 = 12.0;
        tabs.iter()
            .find(|(_, rect)| {
                rect.contains(pos) && pos.x - rect.left() > EDGE && rect.right() - pos.x > EDGE
            })
            .map(|(id, _)| id.clone())
    }

    /// Accent insertion bar marking where a strip drop will land: the gap
    /// before the first tab, between two tabs, or after the last one.
    pub(super) fn paint_strip_insertion(
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
    pub(super) fn drag_title(&self, tab: &Tab) -> String {
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
            Tab::RevisionDiff { .. } | Tab::FileHistory { .. } => tab.git_title(),
            Tab::Vnc {
                label, host, port, ..
            } => {
                if !label.trim().is_empty() {
                    label.clone()
                } else if !host.trim().is_empty() {
                    format!("{host}:{port}")
                } else {
                    "VNC Viewer".into()
                }
            }
        }
    }

    /// Semi-transparent floating preview following the cursor while a
    /// terminal pane is dragged: its title plus a snapshot of its last grid
    /// rows. Painted on the tooltip layer so it floats above splits and
    /// sidebars without intercepting input.
    pub(crate) fn paint_drag_ghost(&self, ui: &mut egui::Ui) {
        let Some(pane) = &self.drag.pane else {
            return;
        };
        // The IDE strip tab is already floating in the dock's drag layer.
        if self.drag.pane_from_strip {
            return;
        }
        // The tab ghost owns the pointer over strip new-tab zones so the
        // two never stack.
        if self.drag.strip_new_tab_hover {
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
            egui::Id::unique("drag-ghost"),
        ));
        let mut job = egui::text::LayoutJob::simple(
            self.drag_title(pane),
            egui::FontId::proportional(13.0),
            appearance::color(&self.theme.text),
            320.0,
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        if !self.drag.pane_snapshot.is_empty() {
            job.wrap.max_rows = 1usize.saturating_add(self.drag.pane_snapshot.len().min(8));
            job.append(
                &format!("\n{}", self.drag.pane_snapshot.join("\n")),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(11.0),
                    color: appearance::color(&self.theme.secondary),
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
        painter.rect_filled(rect, appearance::WIDGET_RADIUS, self.ghost_fill());
        painter.rect_stroke(
            rect,
            appearance::WIDGET_RADIUS,
            egui::Stroke::new(1.5, appearance::color(&self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(rect.min.x + padding.x, rect.min.y + padding.y),
            galley,
            appearance::color(&self.theme.text),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-ghost", rect);
    }

    /// Translucent theme window fill behind drag ghosts.
    fn ghost_fill(&self) -> egui::Color32 {
        appearance::color(&self.theme.window).gamma_multiply(0.92)
    }

    /// Tab-sized ghost following the cursor while a strip tab is dragged to
    /// reorder: the dragged tab's title at real tab size. Painted on the
    /// tooltip layer like the pane ghost, without intercepting input.
    pub(crate) fn paint_tab_ghost(&self, ui: &mut egui::Ui) {
        // Tab reorder drags always show the tab ghost; pane drags show it
        // over strip new-tab zones, where the outcome is a fresh tab.
        if self.drag.tab.is_none() && !(self.drag.pane.is_some() && self.drag.strip_new_tab_hover) {
            return;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos())) else {
            return;
        };
        let title = if let Some(dragged) = &self.drag.tab {
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
        } else if let Some(pane) = &self.drag.pane {
            self.drag_title(pane)
        } else {
            return;
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("tab-ghost"),
        ));
        let mut job = egui::text::LayoutJob::simple(
            title,
            egui::FontId::proportional(13.0),
            appearance::color(&self.theme.text),
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
        painter.rect_filled(rect, appearance::WIDGET_RADIUS, self.ghost_fill());
        painter.rect_stroke(
            rect,
            appearance::WIDGET_RADIUS,
            egui::Stroke::new(1.5, appearance::color(&self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y * 0.5),
            galley,
            appearance::color(&self.theme.text),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "tab-ghost", rect);
    }
}
