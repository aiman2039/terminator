//! Pane zoom in the viewer: a stable scope for zoomable pane bodies, the
//! placeholder left in the dock, and the overlay that paints the body.
use super::super::super::*;
use super::Viewer;
use crate::app::Zoomed;
use crate::native_editor::NativeEditorViewInput;

/// Scope of a main-dock pane body. It is keyed by the view, not the dock
/// leaf, so the zoom overlay paints the very same widgets: terminal focus
/// and selection, and the editor caret, scroll and focus carry over.
pub(crate) fn pane_scope_id(
    project: Option<&str>,
    group: Option<&str>,
    tab: &Tab,
    node: Option<egui_dock::NodePath>,
) -> egui::Id {
    match tab {
        Tab::Terminal(_) => egui::Id::unique(("pane-scope", tab.key())),
        _ => egui::Id::unique(("pane-scope", project, group, tab.key(), node)),
    }
}

impl Viewer<'_> {
    /// Paint a main-dock pane body in its view scope (see [`pane_scope_id`]).
    /// Strip and floating panes cannot zoom and keep their dock scope.
    pub(super) fn pane_scope<R>(
        &mut self,
        ui: &mut egui::Ui,
        tab: &Tab,
        add: impl FnOnce(&mut Self, &mut egui::Ui) -> R,
    ) -> R {
        if self.strip || self.window.is_some() {
            return add(self, ui);
        }
        let id = pane_scope_id(
            self.project.as_deref(),
            self.tab.as_deref(),
            tab,
            self.render_path,
        );
        ui.scope_builder(egui::UiBuilder::new().scope_id(id), |ui| add(self, ui))
            .inner
    }

    /// The dock is rendering the zoomed view: it paints a placeholder,
    /// and the overlay paints the view, once.
    pub(super) fn zoomed_here(&self, tab: &Tab) -> bool {
        if self.strip || self.window.is_some() {
            return false;
        }
        let Some(zoomed) = &self.app.pane_ops.zoomed else {
            return false;
        };
        let source = &zoomed.source;
        zoomed.tab == *tab
            && self.project.as_deref() == Some(source.project.as_str())
            && self.tab.as_deref() == Some(source.group.as_str())
            && (matches!(tab, Tab::Terminal(_))
                || self
                    .render_tab
                    .map_or(self.render_path == Some(source.path.node_path()), |path| {
                        path == source.path
                    }))
    }

    pub(super) fn zoom_placeholder(ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.weak("Zoomed — click to restore");
        });
    }

    /// Zoom on a main-dock leaf tab bar that holds an editor (editors have
    /// no caption row; terminals zoom from theirs). It zooms the leaf's
    /// active pane.
    pub(super) fn editor_zoom_button(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        if self.strip || self.window.is_some() {
            return;
        }
        let editor = self.app.pane_tabs.get(&path).is_some_and(|tabs| {
            tabs.iter()
                .any(|tab| matches!(tab, Tab::NativeEditor { .. }))
        });
        if !editor {
            return;
        }
        let tip = self.app.action_tip("Zoom pane", Action::ZoomPane);
        let response = appearance::sidebar_action(ui, "Maximize2", &tip);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "pane-zoom-editor", response.rect);
        if response.clicked() {
            self.app.queue_zoom(None, Some(path));
        }
    }

    /// "Zoom pane" in a main-dock pane menu (terminals and editors).
    pub(super) fn zoom_menu_item(
        &mut self,
        ui: &mut egui::Ui,
        tab: &Tab,
        pane: egui_dock::NodePath,
    ) {
        if self.strip || self.window.is_some() || !crate::App::zoomable(tab) {
            return;
        }
        let zoomed = self
            .app
            .pane_ops
            .zoomed
            .as_ref()
            .is_some_and(|zoomed| zoomed.tab == *tab);
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: if zoomed { "Restore pane" } else { "Zoom pane" },
            icon: if zoomed { "Minimize2" } else { "Maximize2" },
            shortcut: &self.app.shortcut_label(Action::ZoomPane),
        })
        .clicked()
        {
            self.app.queue_zoom(Some(tab.clone()), Some(pane));
            ui.close();
        }
    }

    /// The zoomed view over the workspace rect `outer`: a dimmed backdrop
    /// and a center card with the view's title and Restore. True when
    /// dismissed by Restore or a click on the backdrop. Esc stays with the
    /// terminal or editor.
    pub(crate) fn zoom_overlay(
        &mut self,
        ctx: &egui::Context,
        outer: egui::Rect,
        zoomed: &Zoomed,
    ) -> bool {
        let card = appearance::center_card_rect(outer);
        let style = appearance::CenterCardStyle::new(&self.app.theme);
        egui::Area::new(egui::Id::unique("pane-zoom"))
            .order(egui::Order::Middle)
            .fixed_pos(outer.min)
            .constrain(false)
            .movable(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(outer);
                ui.set_min_size(outer.size());
                let backdrop = ui.allocate_rect(outer, egui::Sense::click());
                ui.painter()
                    .rect_filled(outer, egui::CornerRadius::ZERO, style.scrim());
                let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card));
                let inner = card.shrink(f32::from(appearance::CENTER_CARD_PADDING));
                let restore = style
                    .frame()
                    .show(&mut card_ui, |ui| {
                        ui.set_min_size(inner.size());
                        ui.set_max_size(inner.size());
                        let restore = self.zoom_title_row(ui, &zoomed.tab);
                        self.zoom_body(ui, zoomed);
                        restore
                    })
                    .inner;
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "pane-zoom-card", card);
                let outside = backdrop.clicked()
                    && backdrop
                        .interact_pointer_pos()
                        .is_some_and(|pos| !card.contains(pos));
                restore || outside
            })
            .inner
    }

    fn zoom_title_row(&mut self, ui: &mut egui::Ui, tab: &Tab) -> bool {
        let title = self.tab_title(&mut tab.clone());
        let tip = self.app.action_tip("Restore pane", Action::ZoomPane);
        ui.horizontal(|ui| {
            ui.label(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let restore = appearance::sidebar_action(ui, "Minimize2", &tip);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "pane-zoom-restore", restore.rect);
                restore.clicked()
            })
            .inner
        })
        .inner
    }

    fn zoom_body(&mut self, ui: &mut egui::Ui, zoomed: &Zoomed) {
        let tab = zoomed.tab.clone();
        match &tab {
            Tab::Terminal(sid) => {
                let Some(session) = self
                    .app
                    .state
                    .sessions
                    .iter()
                    .find(|session| &session.id == sid)
                    .cloned()
                else {
                    return;
                };
                if !self.app.connected {
                    ui.weak("Reconnecting to session daemon…");
                    return;
                }
                self.pane_scope(ui, &tab, |viewer, ui| viewer.terminal_body(ui, &session));
            }
            Tab::NativeEditor { path } => self.pane_scope(ui, &tab, |viewer, ui| {
                viewer.app.native_editor_view(NativeEditorViewInput {
                    ui,
                    path,
                    project: viewer.project.as_deref(),
                    tab: viewer.tab.as_deref(),
                    node: viewer.render_path,
                });
            }),
            _ => {}
        }
    }
}
