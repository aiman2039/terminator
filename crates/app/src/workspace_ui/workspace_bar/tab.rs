use super::super::super::*;
use super::super::tab_menu::WorkspaceTabMenuSpec;
use super::super::tabs::{TabFace, tab_attention_text, tab_tooltip};
use super::{StripActions, StripFrame};
use crate::agent_presence::AttentionCounts;
use crate::workspace::WorkspaceTab;

/// Inputs for painting one strip tab.
pub(super) struct StripTabInput<'a> {
    pub(super) frame: StripFrame<'a>,
    pub(super) group: &'a WorkspaceTab,
    pub(super) index: usize,
    pub(super) count: usize,
    pub(super) layout: (f32, AttentionCounts),
    pub(super) active_id: &'a mut String,
    pub(super) strip_rects: &'a mut Vec<(String, egui::Rect)>,
    pub(super) actions: &'a mut StripActions,
}

/// Per-frame context for one strip tab's paint and input steps.
struct TabPaint<'a> {
    frame: StripFrame<'a>,
    group: &'a WorkspaceTab,
    index: usize,
    count: usize,
    face: TabFace,
    badge: Option<(String, String)>,
    attention: AttentionCounts,
    active: bool,
    rect: egui::Rect,
    response: egui::Response,
}

/// Where the tab's close glyph sits, and what this frame's clicks may act on.
#[derive(Clone, Copy)]
struct TabHits {
    close_rect: egui::Rect,
    close_clicked: bool,
    editing: bool,
}

impl TabPaint<'_> {
    fn label_x(&self) -> f32 {
        if self.face.brand.is_some() {
            36.0
        } else {
            30.0
        }
    }

    fn badge_reserve(&self) -> f32 {
        if self.badge.is_some() { 22.0 } else { 0.0 }
    }
}

impl App {
    pub(super) fn strip_tab(&mut self, ui: &mut egui::Ui, input: StripTabInput<'_>) {
        let StripTabInput {
            frame,
            group,
            index,
            count,
            layout: (width, attention),
            active_id,
            strip_rects,
            actions,
        } = input;
        let face = self.tab_face(Self::group_primary(group));
        let badge = tab_attention_text(attention);
        let active = *active_id == group.id;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::click_and_drag());
        strip_rects.push((group.id.clone(), rect));
        let paint = TabPaint {
            frame,
            group,
            index,
            count,
            face,
            badge,
            attention,
            active,
            rect,
            response,
        };
        self.start_strip_tab_drag(&paint);
        self.preview_hovered_strip_tab(ui, &paint, active_id);
        self.paint_strip_tab_fill(ui, &paint);
        self.paint_strip_tab_identity(ui, &paint);
        let editing = self.paint_strip_tab_label(ui, &paint);
        self.paint_strip_tab_badge(ui, &paint);
        self.paint_strip_tab_underline(ui, &paint);
        let hits = self.strip_tab_close(ui, &paint, editing);
        self.strip_tab_clicks(&paint, hits, actions);
        self.strip_tab_tooltip(&paint);
        paint.response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, paint.active, &paint.face.label)
        });
        self.strip_tab_menu(&paint, actions);
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(
                ui.ctx(),
                &format!("workspace-tab:{}", paint.face.label),
                rect,
            );
            diagnostics::record(
                ui.ctx(),
                &format!("workspace-close:{}", paint.face.label),
                hits.close_rect,
            );
        }
    }

    /// Dragging a strip tab reorders the top-level tabs; pane drags keep
    /// their own payload.
    fn start_strip_tab_drag(&mut self, paint: &TabPaint<'_>) {
        if paint.response.drag_started() && self.drag.pane.is_none() && self.drag.tab.is_none() {
            self.drag.tab = Some(paint.group.id.clone());
        }
    }

    /// Destination preview: hovering a strip tab while dragging a terminal
    /// shows that tab's splits so the drop can target a specific leaf.
    /// Switching back over the origin tab restores it.
    fn preview_hovered_strip_tab(
        &mut self,
        ui: &mut egui::Ui,
        paint: &TabPaint<'_>,
        active_id: &mut String,
    ) {
        let TabPaint {
            frame, group, rect, ..
        } = paint;
        let hovering_tab = frame.drag_same_project
            && ui.input(|i| i.pointer.any_down())
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|pos| rect.contains(pos));
        if !hovering_tab {
            return;
        }
        if self
            .drag
            .preview_origin
            .as_ref()
            .is_some_and(|(p, g)| p == frame.project && g == &group.id)
        {
            self.drag.preview_origin = None;
        } else if self.drag.preview_origin.is_none() && *active_id != group.id {
            self.drag.preview_origin = Some((frame.project.to_owned(), active_id.clone()));
        }
        active_id.clone_from(&group.id);
        ui.painter().rect_stroke(
            *rect,
            0,
            egui::Stroke::new(2.0, appearance::color(&self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    }

    fn paint_strip_tab_fill(&self, ui: &egui::Ui, paint: &TabPaint<'_>) {
        if paint.active && paint.frame.reveal {
            paint.response.scroll_to_me(Some(egui::Align::Center));
        }
        if paint.active || paint.response.hovered() {
            ui.painter().rect_filled(
                paint.rect,
                0,
                if paint.active {
                    appearance::color(&self.theme.window)
                } else {
                    appearance::color(&self.theme.hover)
                },
            );
        }
    }

    /// Terminal color underline, then the focused terminal identity: stable
    /// brand glyph beside the hook lifecycle status or the kind icon.
    fn paint_strip_tab_identity(&self, ui: &mut egui::Ui, paint: &TabPaint<'_>) {
        let TabPaint {
            face, rect, active, ..
        } = paint;
        let terminal_tint = face
            .sid
            .as_deref()
            .map(|sid| self.terminal_title_color(sid));
        if let Some(color) = terminal_tint {
            ui.painter().hline(
                rect.x_range(),
                rect.bottom() - 1.0,
                egui::Stroke::new(if *active { 2.0 } else { 1.0 }, color),
            );
        }
        if let Some(brand) = face.brand {
            appearance::paint_status_icon(appearance::PaintStatusIconInput {
                ui,
                rect: egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 10.0, rect.center().y),
                    egui::vec2(13.0, 13.0),
                ),
                icon: brand,
                tint: appearance::icon_color(),
                spin: false,
            });
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
            appearance::paint_status_icon(appearance::PaintStatusIconInput {
                ui,
                rect: icon_rect,
                icon: status_icon,
                tint,
                spin: state == AgentState::Running,
            });
        } else {
            egui::Image::new(icons::source(face.icon))
                .tint(terminal_tint.unwrap_or(appearance::icon_color()))
                .paint_at(ui, icon_rect);
        }
    }

    /// The tab label, or the inline rename field while renaming. Returns
    /// whether the rename field is showing.
    fn paint_strip_tab_label(&mut self, ui: &mut egui::Ui, paint: &TabPaint<'_>) -> bool {
        let TabPaint {
            face, rect, active, ..
        } = paint;
        let label_x = paint.label_x();
        let badge_reserve = paint.badge_reserve();
        let editing = face
            .sid
            .as_ref()
            .is_some_and(|sid| self.renaming(sid, RenameSurface::Workspace));
        if editing {
            if let Some(sid) = &face.sid {
                self.inline_rename(super::super::menus::InlineRenameInput {
                    ui,
                    sid,
                    surface: RenameSurface::Workspace,
                    rect: egui::Rect::from_min_max(
                        egui::pos2(rect.min.x + label_x, rect.min.y + 8.0),
                        egui::pos2(rect.max.x - (28.0 + badge_reserve), rect.max.y - 7.0),
                    ),
                });
            }
            return true;
        }
        let tint = appearance::color(if *active {
            &self.theme.text
        } else {
            &self.theme.secondary
        });
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
        false
    }

    /// Aggregate attention across the tab's terminals.
    fn paint_strip_tab_badge(&self, ui: &egui::Ui, paint: &TabPaint<'_>) {
        let TabPaint {
            badge,
            rect,
            attention,
            ..
        } = paint;
        let Some((count, _)) = badge else {
            return;
        };
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
            &format!("workspace-tab-attention:{}", paint.face.label),
            badge_rect,
        );
    }

    fn paint_strip_tab_underline(&self, ui: &egui::Ui, paint: &TabPaint<'_>) {
        if !paint.active {
            return;
        }
        let rect = paint.rect;
        // Inset from the bottom edge. A stroke centered on rect.bottom() is
        // clipped away by the strip.
        let underline = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
        );
        ui.painter()
            .rect_filled(underline, 1.0, appearance::color(&self.theme.accent));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("workspace-tab-underline:{}", paint.face.label),
            underline,
        );
    }

    /// The close glyph, or the pin mark on pinned tabs until hovered.
    fn strip_tab_close(&self, ui: &egui::Ui, paint: &TabPaint<'_>, editing: bool) -> TabHits {
        let TabPaint {
            frame,
            group,
            rect,
            response,
            active,
            ..
        } = paint;
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            egui::vec2(20.0, 24.0),
        );
        let project = frame.project;
        let close_response = ui
            .interact(
                close_rect,
                egui::Id::unique(("close-workspace", project, &group.id)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Close tab");
        if group.pinned && !close_response.hovered() {
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "◆",
                egui::FontId::proportional(12.0),
                appearance::color(&self.theme.accent),
            );
        } else if response.hovered() || close_response.hovered() || *active {
            egui::Image::new(icons::source("X"))
                .tint(appearance::icon_color())
                .paint_at(
                    ui,
                    egui::Rect::from_center_size(close_rect.center(), egui::vec2(16.0, 16.0)),
                );
        }
        TabHits {
            close_rect,
            close_clicked: close_response.clicked(),
            editing,
        }
    }

    fn strip_tab_clicks(
        &mut self,
        paint: &TabPaint<'_>,
        hits: TabHits,
        actions: &mut StripActions,
    ) {
        let TabPaint {
            group,
            response,
            face,
            ..
        } = paint;
        let TabHits {
            close_rect,
            close_clicked,
            editing,
        } = hits;
        if close_clicked {
            actions.close = Some(group.id.clone());
        }
        if response.clicked()
            && !editing
            && self.drag.pane.is_none()
            && self.drag.tab.is_none()
            && !close_rect.contains(response.interact_pointer_pos().unwrap_or(egui::Pos2::ZERO))
        {
            actions.switch = Some(group.id.clone());
        }
        if response.double_clicked()
            && !editing
            && self.drag.tab.is_none()
            && let Some(sid) = &face.sid
        {
            self.begin_rename(sid, RenameSurface::Workspace);
        }
    }

    fn strip_tab_tooltip(&self, paint: &TabPaint<'_>) {
        let TabPaint { face, badge, .. } = paint;
        paint
            .response
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
                    && let Some(first) = self.present_session(sid).diagnostics(now()).lines().next()
                {
                    tooltip.push_str(&format!("\n{first}"));
                }
                if let Some((_, breakdown)) = badge {
                    tooltip.push_str(&format!("\nAttention: {breakdown}"));
                }
                ui.label(tooltip);
            });
    }

    fn strip_tab_menu(&mut self, paint: &TabPaint<'_>, actions: &mut StripActions) {
        let TabPaint {
            group,
            index,
            count,
            face,
            response,
            ..
        } = paint;
        appearance::context_menu(response, |ui| {
            let action = self.workspace_tab_menu(
                ui,
                WorkspaceTabMenuSpec {
                    sid: face.sid.as_deref(),
                    index: *index,
                    count: *count,
                    pinned: group.pinned,
                },
            );
            if action.toggle_pin {
                actions.toggle_pin = Some(group.id.clone());
            }
            if action.close {
                actions.close = Some(group.id.clone());
            }
            if action.close_all {
                actions.close_all = true;
            }
            if action.close_left {
                actions.close_left = Some(group.id.clone());
            }
            if action.close_right {
                actions.close_right = Some(group.id.clone());
            }
            if action.add_left {
                actions.add_at = Some(*index);
            }
            if action.add_right {
                actions.add_at = Some(index.saturating_add(1));
            }
        });
    }
}
