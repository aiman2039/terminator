//! Position each newly opened app dialog at its initiating pointer location.
use eframe::egui::{self, Pos2};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct Popups {
    origin: Option<Pos2>,
    visible: HashMap<String, (Pos2, u8)>,
    seen: HashSet<String>,
    reserved: HashMap<String, Option<Pos2>>,
}

impl Popups {
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            if input.pointer.any_pressed() || input.pointer.any_released() {
                self.origin = input.pointer.interact_pos();
            } else if input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
            {
                // Keyboard actions use the current pointer, never an old click.
                self.origin = input.pointer.latest_pos();
            }
        });
    }

    #[cfg(test)]
    pub fn reserve(&mut self, title: &str) {
        self.reserved.insert(title.to_owned(), self.origin);
    }

    pub fn window(&mut self, ctx: &egui::Context, title: &str) -> egui::Window<'static> {
        self.seen.insert(title.to_owned());
        let mut window = egui::Window::new(title.to_owned()).constrain_to(ctx.content_rect());
        let (position, age) = self.visible.entry(title.to_owned()).or_insert_with(|| {
            let origin = self.reserved.remove(title).unwrap_or(self.origin);
            (origin.unwrap_or_else(|| ctx.content_rect().center()), 0)
        });
        // The initial invisible sizing pass uses a provisional size. Reapply
        // the origin once the real size is known, then allow normal dragging.
        if *age < 2 {
            window = window
                // egui 0.36 title-bar dragging restores the previous area position.
                // Disable it only during placement so current_pos takes effect.
                .movable(false)
                .current_pos(*position + egui::vec2(8.0, 8.0));
        }
        window
    }

    pub fn centered(&mut self, ctx: &egui::Context, title: &str) -> egui::Window<'static> {
        self.seen.insert(title.to_owned());
        egui::Window::new(title.to_owned())
            .constrain_to(ctx.content_rect())
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
    }

    pub fn end_frame(&mut self) {
        self.visible.retain(|title, (_, age)| {
            *age = age.saturating_add(1);
            self.seen.contains(title)
        });
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(ctx: &egui::Context, popups: &mut Popups, title: &str) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            rect = popups
                .window(ctx, title)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Keep running or cancel");
                })
                .unwrap()
                .response
                .rect;
        });
        output.textures_delta.clear();
        popups.end_frame();
        rect
    }

    #[test]
    fn reopening_moves_to_new_click_and_stays_when_pointer_moves() {
        let ctx = egui::Context::default();
        let mut popups = Popups {
            origin: Some(egui::pos2(200.0, 150.0)),
            ..Default::default()
        };
        let _ = draw(&ctx, &mut popups, "Close tab?");
        let first = draw(&ctx, &mut popups, "Close tab?");
        assert!(first.min.distance(egui::pos2(208.0, 158.0)) < 2.0);
        popups.origin = Some(egui::pos2(400.0, 300.0));
        let stable = draw(&ctx, &mut popups, "Close tab?");
        assert!(stable.min.distance(first.min) < 2.0);
        popups.end_frame();
        let reopened = draw(&ctx, &mut popups, "Close tab?");
        assert!(reopened.min.distance(egui::pos2(408.0, 308.0)) < 2.0);
    }

    #[test]
    fn delayed_editor_confirmation_retains_origin_and_is_constrained() {
        let ctx = egui::Context::default();
        let mut popups = Popups {
            origin: Some(egui::pos2(790.0, 590.0)),
            ..Default::default()
        };
        popups.reserve("Close file");
        popups.origin = Some(egui::pos2(20.0, 20.0));
        let _ = draw(&ctx, &mut popups, "Close file");
        let rect = draw(&ctx, &mut popups, "Close file");
        assert!(rect.left() > 400.0 && rect.top() > 300.0, "{rect:?}");
        assert!(ctx.content_rect().contains_rect(rect), "{rect:?}");
    }

    fn draw_centered(ctx: &egui::Context, popups: &mut Popups, title: &str) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            rect = popups
                .centered(ctx, title)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.set_width(300.0);
                    ui.label("Choose a folder to begin.");
                    ui.add_sized([300.0, 28.0], egui::Button::new("Choose project folder"));
                    ui.add_sized([300.0, 28.0], egui::Button::new("Skip"));
                })
                .unwrap()
                .response
                .rect;
        });
        output.textures_delta.clear();
        popups.end_frame();
        rect
    }

    #[test]
    fn first_project_dialog_is_centered() {
        let ctx = egui::Context::default();
        let mut popups = Popups::default();
        let _ = draw_centered(&ctx, &mut popups, "Choose your first project");
        let rect = draw_centered(&ctx, &mut popups, "Choose your first project");
        let center = ctx.content_rect().center();
        assert!(
            rect.center().distance(center) < 12.0,
            "dialog center {rect:?} vs content {center:?}"
        );
    }
}
