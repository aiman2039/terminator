//! The strip's "All tabs" button: every top-level tab of the project in
//! strip order, with a filter, so a tab scrolled out of view is one click
//! away. A dot on the button shows a scrolled-out tab that needs attention.
use super::super::super::*;
use super::super::tabs::tab_attention_text;
use super::StripActions;
use crate::agent_presence::AttentionCounts;

/// One row of the All tabs list.
struct AllTabsRow {
    id: String,
    label: String,
    icon: &'static str,
    attention: AttentionCounts,
    active: bool,
}

/// Inputs for [`App::strip_all_tabs_menu`].
pub(super) struct AllTabsInput<'a> {
    pub(super) workspace: &'a Workspace,
    pub(super) tab_layout: &'a [(f32, AttentionCounts)],
    /// Attention of the tabs outside the scroll viewport; `Some` paints a
    /// dot on the button.
    pub(super) hidden: Option<AttentionCounts>,
    pub(super) actions: &'a mut StripActions,
}

/// Height of one All tabs row.
const ROW_HEIGHT: f32 = 24.0;

impl App {
    /// The All tabs button and its list.
    pub(super) fn strip_all_tabs_menu(&self, ui: &mut egui::Ui, input: AllTabsInput<'_>) {
        let AllTabsInput {
            workspace,
            tab_layout,
            hidden,
            actions,
        } = input;
        let rows: Vec<AllTabsRow> = workspace
            .tabs
            .iter()
            .zip(tab_layout)
            .map(|(group, (_, attention))| {
                let face = self.tab_face(Self::group_primary(group));
                AllTabsRow {
                    id: group.id.clone(),
                    label: face.label,
                    icon: face.status.map_or(face.icon, |(_, icon, _)| icon),
                    attention: *attention,
                    active: group.id == workspace.active,
                }
            })
            .collect();
        let hidden_attention = hidden;
        let response = appearance::icon_menu_button(ui, "List", |ui| {
            self.all_tabs_list(ui, &rows, actions);
        })
        .response
        .on_hover_text("All tabs");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "workspace-all-tabs", response.rect);
        if let Some(attention) = hidden_attention {
            ui.painter().circle_filled(
                egui::pos2(response.rect.right() - 5.0, response.rect.top() + 5.0),
                3.5,
                self.attention_tint(attention),
            );
        }
    }

    fn attention_tint(&self, attention: AttentionCounts) -> egui::Color32 {
        appearance::color(if attention.waiting() > 0 {
            &self.theme.status_waiting
        } else {
            &self.theme.status_failed
        })
    }

    fn all_tabs_list(&self, ui: &mut egui::Ui, rows: &[AllTabsRow], actions: &mut StripActions) {
        let filter_id = egui::Id::unique("workspace-all-tabs-filter");
        ui.set_width(300.0);
        let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
        let field = ui.add(
            appearance::singleline(&mut filter)
                .hint_text("Filter tabs")
                .desired_width(f32::INFINITY),
        );
        // The field is the list's only text input: typing filters at once.
        if !field.has_focus() {
            field.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
        let mut choose = |ui: &mut egui::Ui, id: &str| {
            actions.switch = Some(id.to_owned());
            ui.data_mut(|d| d.remove::<String>(filter_id));
            ui.close();
        };
        let query = filter.trim();
        let shown: Vec<&AllTabsRow> = rows
            .iter()
            .filter(|row| {
                query.is_empty() || !appearance::match_positions(&row.label, query).is_empty()
            })
            .collect();
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter && let Some(first) = shown.first() {
            choose(ui, &first.id);
            return;
        }
        if shown.is_empty() {
            ui.weak("No tab matches");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in shown {
                    if self.all_tabs_row(ui, row, query) {
                        choose(ui, &row.id);
                    }
                }
            });
    }

    /// One row: glyph, label (typed match highlighted) and attention count.
    /// Returns true on a click.
    fn all_tabs_row(&self, ui: &mut egui::Ui, row: &AllTabsRow, query: &str) -> bool {
        let response = appearance::popup_row(ui, ROW_HEIGHT, row.active);
        let rect = response.rect;
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: egui::Rect::from_center_size(
                egui::pos2(rect.left() + 14.0, rect.center().y),
                egui::Vec2::splat(appearance::TOOLBAR_ICON),
            ),
            icon: row.icon,
            size: appearance::TOOLBAR_ICON,
            tint: appearance::icon_color(),
        });
        let font = egui::FontId::proportional(appearance::FONT_SMALL);
        let job = appearance::highlighted_label(appearance::HighlightedLabelInput {
            text: &row.label,
            query,
            font: font.clone(),
            color: ui.visuals().text_color(),
            matched: appearance::popup_match_color(ui.visuals()),
        });
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(
            egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
            galley,
            ui.visuals().text_color(),
        );
        if let Some((count, breakdown)) = tab_attention_text(row.attention) {
            ui.painter().text(
                egui::pos2(rect.right() - 8.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                count,
                font,
                self.attention_tint(row.attention),
            );
            return response.on_hover_text(breakdown).clicked();
        }
        response.clicked()
    }
}

/// Aggregate attention of the tabs whose rect is not fully inside the strip
/// viewport, or `None` when every such tab is quiet.
pub(super) fn hidden_attention(
    viewport: egui::Rect,
    strip_rects: &[(String, egui::Rect)],
    tab_layout: &[(f32, AttentionCounts)],
) -> Option<AttentionCounts> {
    let mut total = AttentionCounts::default();
    for ((_, rect), (_, attention)) in strip_rects.iter().zip(tab_layout) {
        let inside = rect.left() >= viewport.left() - 0.5 && rect.right() <= viewport.right() + 0.5;
        if !inside {
            total.input = total.input.saturating_add(attention.input);
            total.permission = total.permission.saturating_add(attention.permission);
            total.failed = total.failed.saturating_add(attention.failed);
        }
    }
    (!total.is_empty()).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_attention_counts_only_tabs_outside_the_viewport() {
        let rect = |left: f32| {
            egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(left + 50.0, 30.0))
        };
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 30.0));
        let rects = [
            ("in".to_owned(), rect(0.0)),
            ("out".to_owned(), rect(120.0)),
        ];
        let waiting = AttentionCounts {
            input: 1,
            ..AttentionCounts::default()
        };
        let quiet = AttentionCounts::default();
        let hidden = |layout: &[(f32, AttentionCounts)]| hidden_attention(viewport, &rects, layout);
        assert_eq!(hidden(&[(50.0, waiting), (50.0, quiet)]), None);
        assert_eq!(hidden(&[(50.0, quiet), (50.0, waiting)]), Some(waiting));
    }
}
