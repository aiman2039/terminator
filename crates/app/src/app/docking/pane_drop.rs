//! Caption-initiated pane drags inside the main dock: hit-testing the drop
//! target, painting the landing preview, and moving the pane on release.

use crate::app::LandStripShellOnLeafInput;
use crate::*;

struct PaintLandingPreviewInput<'a> {
    ui: &'a mut egui::Ui,
    dock: &'a Workspace,
    pane: &'a Tab,
    target: Option<egui_dock::NodePath>,
    zone: Option<PaneDropZone>,
}

/// Where a dragged pane would land under the pointer this frame.
struct PaneDrop {
    pane: Tab,
    /// The hovered split leaf and its rect.
    target: Option<(egui_dock::NodePath, egui::Rect)>,
    /// Caption slot when hovering a leaf's tab bar.
    tab_insert: Option<egui_dock::TabIndex>,
    zone: Option<PaneDropZone>,
}

impl PaneDrop {
    fn insertion(&self) -> egui_dock::TabInsert {
        self.tab_insert
            .map(egui_dock::TabInsert::Insert)
            .unwrap_or_else(|| {
                self.zone
                    .and_then(PaneDropZone::split)
                    .map_or(egui_dock::TabInsert::Append, egui_dock::TabInsert::Split)
            })
    }

    /// Moves a pane dragged within the dock (no view source, not from the
    /// strip) into leaf `path` of `group`.
    fn move_within(&self, dock: &mut Workspace, group: &str, path: egui_dock::NodePath) -> bool {
        let pane = &self.pane;
        match self.zone {
            Some(PaneDropZone::Center) | None => {
                if dock.find_tab(pane).is_some() {
                    // Same group: `move_pane_to_leaf` also focuses a
                    // drop back onto the pane's own leaf.
                    dock.move_pane_to_leaf(pane, path)
                } else {
                    dock.move_pane_to_group_leaf(pane, group, path)
                }
            }
            Some(edge) => {
                // `move_pane_to_split` focuses a lone pane dropped on
                // an edge of its own leaf instead of splitting it.
                edge.split().is_some_and(|split| {
                    dock.move_pane_to_split(crate::workspace::MovePaneToSplitInput {
                        pane,
                        dest_group: group,
                        dest: path,
                        split,
                    })
                })
            }
        }
    }
}

impl App {
    /// Drop zone within a hovered split leaf: the middle swaps or joins,
    /// while a band near an edge opens the dragged pane in a new split
    /// beside that leaf.
    fn pane_drop_zone(rect: egui::Rect, pos: egui::Pos2) -> PaneDropZone {
        let band = (rect.width().min(rect.height()) * 0.25).clamp(20.0, 96.0);
        let top = pos.y - rect.top();
        let bottom = rect.bottom() - pos.y;
        let left = pos.x - rect.left();
        let right = rect.right() - pos.x;
        if top <= band && top <= bottom && top <= left && top <= right {
            PaneDropZone::Above
        } else if bottom <= band && bottom <= top && bottom <= left && bottom <= right {
            PaneDropZone::Below
        } else if left <= band && left <= top && left <= bottom && left <= right {
            PaneDropZone::Left
        } else if right <= band && right <= top && right <= bottom && right <= left {
            PaneDropZone::Right
        } else {
            PaneDropZone::Center
        }
    }

    /// Complete a caption-initiated pane drag. Releasing over a split leaf of
    /// the previewed top-level tab lands the pane there (single panes swap,
    /// otherwise the dragged pane joins the leaf, including across tabs);
    /// releasing elsewhere cancels and switches back to the origin tab. The
    /// workspace strip runs earlier in the frame and consumes releases over
    /// its own tabs.
    pub(crate) fn finish_pane_drop(&mut self, ui: &mut egui::Ui, dock: &mut Workspace) {
        let Some(pane) = self.drag.pane.clone() else {
            return;
        };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_pane_drop(dock);
            return;
        }
        let dragging = ui.input(|i| i.pointer.any_down());
        let released = ui.input(|i| i.pointer.any_released());
        if !dragging && !released {
            self.cancel_pane_drop(dock);
            return;
        }
        let pos = ui.input(|i| i.pointer.interact_pos());
        let Some(pos) = pos else {
            if released {
                self.cancel_pane_drop(dock);
            }
            return;
        };
        let drop = self.pane_drop_at(dock, pane, pos);
        if dragging && !released {
            self.preview_pane_drop(ui, dock, &drop);
            return;
        }
        self.release_pane_drop(dock, &drop);
    }

    fn cancel_pane_drop(&mut self, dock: &mut Workspace) {
        self.revert_drop_preview(dock);
        self.end_pane_drag();
    }

    fn pane_drop_at(&self, dock: &Workspace, pane: Tab, pos: egui::Pos2) -> PaneDrop {
        let target = dock
            .iter_leaves()
            .find(|(_, leaf)| leaf.rect.contains(pos))
            .map(|(path, leaf)| (path, leaf.rect));
        let tab_insert = self.drag.pane_source.as_ref().and_then(|_| {
            self.drag
                .drop_tabs
                .iter()
                .find(|(_, rect)| rect.contains(pos))
                .map(|(path, rect)| {
                    egui_dock::TabIndex(
                        path.tab
                            .0
                            .saturating_add(usize::from(pos.x >= rect.center().x)),
                    )
                })
        });
        let zone = target.map(|(_, rect)| {
            if tab_insert.is_some() {
                PaneDropZone::Center
            } else {
                Self::pane_drop_zone(rect, pos)
            }
        });
        PaneDrop {
            pane,
            target,
            tab_insert,
            zone,
        }
    }

    fn preview_pane_drop(&self, ui: &mut egui::Ui, dock: &Workspace, drop: &PaneDrop) {
        self.paint_landing_preview(PaintLandingPreviewInput {
            ui,
            dock,
            pane: &drop.pane,
            target: drop.target.map(|(path, _)| path),
            zone: drop.zone,
        });
        if drop.target.is_none() && self.drag.strip_tab_hover {
            self.preview_strip_tab_landing(ui, dock, &drop.pane);
        }
        ui.ctx().request_repaint();
    }

    /// Hovering a strip tab interior previews the move-into outcome
    /// at real size: wash the previewed tab's focused leaf, where a
    /// release would land the pane.
    fn preview_strip_tab_landing(&self, ui: &mut egui::Ui, dock: &Workspace, pane: &Tab) {
        let path = dock
            .main_surface()
            .focused_leaf()
            .map(|node| egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            });
        let landed = path.filter(|path| {
            dock.leaf(*path)
                .map(|leaf| leaf.rect.width() > 1.0 && leaf.rect.height() > 1.0)
                .unwrap_or(false)
        });
        let Some(path) = landed else {
            return;
        };
        #[cfg(feature = "test-support")]
        if let Ok(leaf) = dock.leaf(path) {
            diagnostics::record(ui.ctx(), "strip-drop-wash", leaf.rect);
        }
        self.paint_landing_preview(PaintLandingPreviewInput {
            ui,
            dock,
            pane,
            target: Some(path),
            zone: Some(PaneDropZone::Center),
        });
    }

    fn release_pane_drop(&mut self, dock: &mut Workspace, drop: &PaneDrop) {
        if let Some((path, _)) = drop.target {
            if self.move_dropped_pane(dock, drop, path) {
                self.pane_drop_landed(dock, &drop.pane);
            } else {
                self.revert_drop_preview(dock);
            }
        } else {
            self.revert_drop_preview(dock);
        }
        // A release the strip did not consume ends the drag here; the
        // post-dock checkout in `workspace_project` clears leftovers.
        self.end_pane_drag();
    }

    fn move_dropped_pane(
        &mut self,
        dock: &mut Workspace,
        drop: &PaneDrop,
        path: egui_dock::NodePath,
    ) -> bool {
        let group = dock.active.clone();
        let from_strip = self.drag.pane_from_strip;
        if let Some(source) = self.drag.pane_source.clone() {
            self.selected.as_ref() == Some(&source.project)
                && dock.move_view_to_leaf(crate::workspace::MoveViewToLeafInput {
                    source: &source,
                    pane: &drop.pane,
                    group: &group,
                    dest: path,
                    insertion: drop.insertion(),
                })
        } else if from_strip {
            match &drop.pane {
                Tab::Terminal(sid) => self.land_strip_shell_on_leaf(LandStripShellOnLeafInput {
                    dock,
                    sid,
                    group: &group,
                    path,
                    zone: drop.zone.unwrap_or(PaneDropZone::Center),
                }),
                _ => false,
            }
        } else {
            drop.move_within(dock, &group, path)
        }
    }

    fn pane_drop_landed(&mut self, dock: &Workspace, pane: &Tab) {
        if self.drag.pane_source.is_some() {
            if let Some(project) = self.selected.clone() {
                self.focus_moved_view(pane, &project, dock);
            }
            self.pending_layout_save = true;
        }
        if let Tab::Terminal(sid) = pane {
            self.active_session = Some(sid.clone());
            self.focus_tab = Some(pane.clone());
        }
        self.pane_index = None;
        self.drag.preview_origin = None;
    }

    /// Switch back to the tab a cancelled drag started from. Previewing never
    /// moves panes, so the origin group always still exists.
    pub(crate) fn revert_drop_preview(&mut self, dock: &mut Workspace) {
        if let Some((_, origin)) = self.drag.preview_origin.take()
            && dock.tabs.iter().any(|tab| tab.id == origin)
        {
            dock.active = origin;
        }
    }

    /// Landing preview for the hovered split leaf: a translucent accent wash
    /// over exactly where the dragged pane will land (the whole leaf, or the
    /// edge half a split drop would open), with a divider on the future
    /// split boundary. A center drop between two single panes exchanges
    /// them, so the source leaf is outlined as well.
    fn paint_landing_preview(&self, input: PaintLandingPreviewInput<'_>) {
        let PaintLandingPreviewInput {
            ui,
            dock,
            pane,
            target,
            zone,
        } = input;
        let (Some(path), Some(zone)) = (target, zone) else {
            return;
        };
        let Ok(leaf) = dock.leaf(path) else {
            return;
        };
        let accent = appearance::color(&self.theme.accent);
        let wash = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36);
        let landing = zone.landing(leaf.rect);
        ui.painter().rect_filled(landing, 2, wash);
        ui.painter().rect_stroke(
            landing,
            2,
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Inside,
        );
        if zone != PaneDropZone::Center {
            // Divider where the new split boundary will appear.
            let divider = match zone {
                PaneDropZone::Above => [landing.left_bottom(), landing.right_bottom()],
                PaneDropZone::Below => [landing.left_top(), landing.right_top()],
                PaneDropZone::Left => [landing.right_top(), landing.right_bottom()],
                PaneDropZone::Right => [landing.left_top(), landing.left_bottom()],
                PaneDropZone::Center => return,
            };
            ui.painter()
                .line_segment(divider, egui::Stroke::new(2.0, accent));
        }
        // A center swap keeps every split in place and only exchanges two
        // panes: outline the other side and say so.
        let source = dock.find_tab(pane).map(egui_dock::TabPath::node_path);
        let mut swapping = false;
        if zone == PaneDropZone::Center
            && let Some(node) = source
            && node != path
            && let (Ok(from), Ok(to)) = (dock.leaf(node), dock.leaf(path))
            && from.tabs.len() == 1
            && to.tabs.len() == 1
        {
            swapping = true;
            ui.painter().rect_stroke(
                from.rect,
                2,
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            egui::pos2(landing.min.x + 8.0, landing.min.y + 6.0),
            egui::Align2::LEFT_TOP,
            if swapping {
                "Swap terminals"
            } else {
                zone.label()
            },
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220),
        );
    }
}
