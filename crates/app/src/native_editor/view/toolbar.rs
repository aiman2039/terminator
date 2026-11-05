//! The Markdown floating toolbar: the format buttons, the heading menu,
//! and Edit / Preview / Split in the top-right corner of the editor area,
//! like the `JetBrains` floating toolbar. It fades out while the buffer
//! changes under the user's typing, so it never covers the text being
//! written, and comes back after a pause or when the pointer is on it.
use super::*;

/// Gap to the editor area's right and top edges. The right gap keeps the
/// scroll rail beside the text visible.
const INSET: egui::Vec2 = egui::vec2(18.0, 6.0);
/// Seconds after the last edit before the toolbar comes back.
pub(super) const TYPING_PAUSE: f64 = 1.5;
/// Fade time, in seconds.
const FADE: f32 = 0.15;

impl ViewPass<'_> {
    /// Float the toolbar over the top-right corner of `area` (the editor
    /// area under the in-file bars). It sits on a layer just above the
    /// pane, so it paints over the source view and the preview.
    pub(super) fn markdown_toolbar(&mut self, ui: &egui::Ui, area: egui::Rect) {
        let id = egui::Id::unique(("native-markdown-toolbar", &self.frame.source_id));
        let (visible, shown) = self.toolbar_fade(ui, id);
        if shown <= 0.0 {
            return;
        }
        let layer = ui.layer_id();
        let toolbar = egui::Area::new(id)
            .order(layer.order)
            .fixed_pos(egui::pos2(area.right() - INSET.x, area.top() + INSET.y))
            .pivot(egui::Align2::RIGHT_TOP)
            .constrain_to(area)
            .interactable(visible)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                ui.set_opacity(shown);
                toolbar_frame(ui)
                    .show(ui, |ui| ui.horizontal(|ui| self.toolbar_row(ui)).inner)
                    .inner
            });
        ui.ctx().set_sublayer(layer, toolbar.response.layer_id);
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "native-markdown-toolbar", toolbar.response.rect);
        let (heading_jump, formatted) = toolbar.inner;
        if formatted {
            // The toolbar's own edit is not typing: the toolbar stays.
            let revision = self.doc.doc.as_ref().map_or(0, Doc::revision);
            ui.ctx().data_mut(|data| {
                data.insert_temp(id.with("edit"), (revision, f64::NEG_INFINITY));
            });
        }
        self.jump_to_heading(ui, heading_jump);
    }

    /// The toolbar hides while the buffer changes (the user types), unless
    /// the pointer is on it. Returns whether it shows, and its opacity.
    fn toolbar_fade(&self, ui: &egui::Ui, id: egui::Id) -> (bool, f32) {
        let ctx = ui.ctx();
        let now = ui.input(|input| input.time);
        let revision = self.doc.doc.as_ref().map_or(0, Doc::revision);
        let edited = ctx.data_mut(|data| {
            let seen = data.get_temp_mut_or(id.with("edit"), (revision, f64::NEG_INFINITY));
            if seen.0 != revision {
                *seen = (revision, now);
            }
            seen.1
        });
        let since = now - edited;
        let typing = since < TYPING_PAUSE;
        if typing {
            let wait = (TYPING_PAUSE - since).max(0.0);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        let hovered = egui::AreaState::load(ctx, id)
            .zip(ctx.pointer_hover_pos())
            .is_some_and(|(area, pointer)| area.rect().contains(pointer));
        let visible = !typing || hovered;
        (
            visible,
            ctx.animate_bool_with_time(id.with("fade"), visible, FADE),
        )
    }

    /// Format buttons (while the source is editable), the heading menu,
    /// then the modes at the right end, so they keep their place when
    /// Preview hides the format buttons. Returns a picked heading line and
    /// whether a format button edited the buffer.
    fn toolbar_row(&mut self, ui: &mut egui::Ui) -> (Option<usize>, bool) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut formatted = false;
        if !self.read_only() && self.doc.markdown_mode != markdown::Mode::Preview {
            formatted = self.markdown_format(ui);
            ui.separator();
        }
        let heading_jump = self.heading_menu(ui);
        ui.separator();
        self.markdown_modes(ui);
        (heading_jump, formatted)
    }

    fn markdown_modes(&mut self, ui: &mut egui::Ui) {
        for (option, icon) in [
            (markdown::Mode::Edit, "Pencil"),
            (markdown::Mode::Preview, "Eye"),
            (markdown::Mode::Split, "Columns2"),
        ] {
            let response = appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon,
                tip: option.label(),
                selected: self.doc.markdown_mode == option,
            });
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(
                ui.ctx(),
                &format!("native-markdown-mode:{}", option.label()),
                response.rect,
            );
            if response.clicked() {
                self.doc.markdown_mode = option;
                if option == markdown::Mode::Edit {
                    ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
                }
            }
        }
    }

    fn heading_menu(&self, ui: &mut egui::Ui) -> Option<usize> {
        let mut heading_jump = None;
        appearance::icon_menu_button(ui, "ListTree", |ui| {
            heading_jump = Self::heading_items(ui, self.doc.doc.as_ref());
        })
        .response
        .on_hover_text("Go to heading");
        heading_jump
    }

    /// One button per Markdown heading; returns the clicked heading's line.
    fn heading_items(ui: &mut egui::Ui, buffer: Option<&Doc>) -> Option<usize> {
        let buffer = buffer?;
        let mut jump = None;
        let mut found = false;
        for block in parse_blocks(buffer) {
            let BlockKind::Heading(level) = block.kind else {
                continue;
            };
            found = true;
            let title = buffer.line_text(block.start_line);
            let title = title.trim_start_matches('#').trim();
            let label = format!(
                "{}{title}",
                "  ".repeat(usize::from(level).saturating_sub(1))
            );
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: &label,
                icon: "ListTree",
                shortcut: "",
            })
            .clicked()
            {
                jump = Some(block.start_line);
                ui.close();
            }
        }
        if !found {
            ui.weak("No headings");
        }
        jump
    }

    /// A picked heading opens the source at its line.
    fn jump_to_heading(&mut self, ui: &egui::Ui, heading_jump: Option<usize>) {
        if let Some(line) = heading_jump
            && let Some(buffer) = self.doc.doc.as_mut()
        {
            self.doc.markdown_mode = markdown::Mode::Edit;
            self.doc.engine.place_cursor(buffer, line, 0);
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }
}

/// A small raised panel in the theme's window colors.
fn toolbar_frame(ui: &egui::Ui) -> egui::Frame {
    let visuals = ui.visuals();
    egui::Frame::new()
        .fill(visuals.window_fill)
        .stroke(visuals.window_stroke)
        .corner_radius(appearance::WIDGET_RADIUS)
        .inner_margin(2)
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(48),
        })
}
