//! Per-frame fixture setup: scale, window level and size, scripted typing,
//! and repaint scheduling.
use super::Diagnostics;
use eframe::egui;
use std::time::Duration;

impl Diagnostics {
    pub fn frame(&mut self, ctx: &egui::Context) {
        if self.path.is_none() {
            return;
        }
        if !self.scale_configured {
            self.configure_window(ctx);
        }
        if self.scale_configured && !self.sized {
            self.size_window(ctx);
        }
        if std::env::var_os("TERMINATOR_TEST_INPUT").is_some() {
            self.scripted_editor_input(ctx);
        }
        self.schedule_repaint(ctx);
    }

    fn configure_window(&mut self, ctx: &egui::Context) {
        let scale = std::env::var("TERMINATOR_TEST_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| (1.0..=2.0).contains(s))
            .unwrap_or(1.0);
        self.start_ticking(ctx);
        ctx.set_pixels_per_point(scale);
        let background = std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some();
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            if background && std::env::var_os("TERMINATOR_TEST_VISIBLE_CAPTURE").is_none() {
                egui::WindowLevel::AlwaysOnBottom
            } else {
                egui::WindowLevel::AlwaysOnTop
            },
        ));
        if background && std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_none() {
            // Synthetic fixtures must not intercept the user's real wheel or clicks.
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
        }
        self.scale_configured = true;
        if std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        crate::console::Console::err(format_args!("Native fixture initialized at scale {scale}"));
    }

    /// Repaints on a timer so scripted actions run without real input.
    /// Passive captures do not tick.
    fn start_ticking(&self, ctx: &egui::Context) {
        self.ticking.store(
            std::env::var_os("TERMINATOR_TEST_PASSIVE_CAPTURE").is_none(),
            std::sync::atomic::Ordering::Relaxed,
        );
        let ticking = std::sync::Arc::clone(&self.ticking);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            while ticking.load(std::sync::atomic::Ordering::Relaxed) {
                repaint.request_repaint();
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }

    /// Sizes the window once the requested scale has taken effect.
    fn size_window(&mut self, ctx: &egui::Context) {
        let scale = std::env::var("TERMINATOR_TEST_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0);
        if (ctx.pixels_per_point() - scale).abs() >= 0.01 {
            return;
        }
        let custom_size = std::env::var("TERMINATOR_TEST_SIZE")
            .ok()
            .and_then(|s| serde_json::from_str::<[f32; 2]>(&s).ok())
            .filter(|s| {
                s.iter().all(|n| n.is_finite())
                    && (450.0..=2000.0).contains(&s[0])
                    && (275.0..=1200.0).contains(&s[1])
            });
        let size = if let Some([width, height]) = custom_size {
            egui::vec2(width, height)
        } else if std::env::var_os("TERMINATOR_TEST_NARROW").is_some() {
            egui::vec2(900.0, 650.0)
        } else {
            egui::vec2(1440.0, 900.0)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
            450.0, 275.0,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.sized = true;
    }

    /// Types into the focused editor: insert text, leave insert mode, save.
    fn scripted_editor_input(&mut self, ctx: &egui::Context) {
        let ms = self.started.elapsed().as_millis();
        let event = match self.input_phase {
            0 if ms > 1200 => Some(egui::Event::Text("iUI_INSERT ".into())),
            1 if ms > 1500 => Some(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }),
            2 if ms > 1800 => Some(egui::Event::Text(":w\r".into())),
            _ => None,
        };
        if let Some(event) = event {
            ctx.input_mut(|i| {
                i.focused = true;
                i.events.push(event);
            });
            self.input_phase = self.input_phase.saturating_add(1);
        }
    }

    fn schedule_repaint(&self, ctx: &egui::Context) {
        if std::env::var_os("TERMINATOR_TEST_PASSIVE_CAPTURE").is_some()
            && self.started.elapsed() > Duration::from_secs(3)
        {
            let deadline = Duration::from_millis(
                std::env::var("TERMINATOR_CAPTURE_AFTER_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3000),
            );
            ctx.request_repaint_after(
                deadline
                    .saturating_sub(self.started.elapsed())
                    .saturating_add(Duration::from_millis(1)),
            );
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
