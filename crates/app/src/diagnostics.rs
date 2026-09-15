//! Opt-in renderer capture for native integration tests. Never enabled in normal builds.
use eframe::egui;
use std::time::{Duration, Instant};
pub struct Diagnostics {
    started: Instant,
    requested: bool,
    scale_configured: bool,
    sized: bool,
    input_phase: u8,
    path: Option<std::path::PathBuf>,
    actions: Vec<FixtureAction>,
    action_index: usize,
    release: Option<(egui::Pos2, egui::PointerButton)>,
    pointer: Option<egui::Pos2>,
    ticking: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            requested: false,
            scale_configured: false,
            sized: false,
            input_phase: 0,
            path: std::env::var_os("TERMINATOR_CAPTURE_PATH").map(Into::into),
            actions: std::env::var("TERMINATOR_TEST_ACTIONS")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            action_index: 0,
            release: None,
            pointer: None,
            ticking: Default::default(),
        }
    }
}
impl Diagnostics {
    pub fn input(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.path.is_none() {
            return;
        }
        if let Some(path) = std::env::var_os("TERMINATOR_TEST_ACTIONS_PATH")
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(actions) = serde_json::from_slice::<Vec<FixtureAction>>(&bytes)
        {
            self.actions = actions;
        }
        // A fixture owns its input. Do not let concurrent desktop typing or
        // clipboard shortcuts reach its shells/editors or leak into captures.
        if std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_none() {
            input.events.retain(|event| {
                matches!(
                    event,
                    egui::Event::Screenshot { .. } | egui::Event::WindowFocused(_)
                )
            });
        }
        if std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some() {
            input.focused = true;
        }
        if let Some(pos) = self.pointer {
            input.events.push(egui::Event::PointerMoved(pos));
        }
        if let Some((pos, button)) = self.release.take() {
            input.events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: Default::default(),
            });
        } else if let Some(action) = self.actions.get(self.action_index)
            && self.started.elapsed().as_millis() >= action.at_ms as u128
            && let Some(rect) = ctx.data(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", &action.target)))
            })
        {
            let pos = if action.hover {
                rect.min + egui::vec2(45.0, 10.0)
            } else {
                rect.center()
            };
            self.pointer = Some(pos);
            input.focused = true;
            input.events.push(egui::Event::PointerMoved(pos));
            if let Some(delta) = action.scroll {
                input.events.push(egui::Event::MouseWheel {
                    unit: match action.wheel_unit.as_deref() {
                        Some("line") => egui::MouseWheelUnit::Line,
                        Some("page") => egui::MouseWheelUnit::Page,
                        _ => egui::MouseWheelUnit::Point,
                    },
                    phase: match action.wheel_phase.as_deref() {
                        Some("start") => egui::TouchPhase::Start,
                        Some("end") => egui::TouchPhase::End,
                        Some("cancel") => egui::TouchPhase::Cancel,
                        _ => egui::TouchPhase::Move,
                    },
                    delta: egui::vec2(action.scroll_x.unwrap_or_default(), delta),
                    modifiers: egui::Modifiers {
                        shift: action.shift,
                        ..Default::default()
                    },
                });
            } else if let Some(key) = &action.key {
                if let Some(key) = egui::Key::from_name(key) {
                    input.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    });
                }
            } else if let Some(text) = &action.input {
                input.events.push(egui::Event::Text(text.clone()));
            } else if let Some(text) = &action.text {
                let modifiers = egui::Modifiers {
                    command: true,
                    mac_cmd: cfg!(target_os = "macos"),
                    ctrl: !cfg!(target_os = "macos"),
                    ..Default::default()
                };
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
                input.events.push(egui::Event::Text(text.clone()));
            } else if !action.hover {
                let button = if action.right_click {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                };
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: Default::default(),
                });
                self.release = Some((pos, button));
            }
            if action.capture {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            eprintln!("Fixture action: {}", action.target);
            self.action_index += 1;
        }
    }
    pub fn actions_completed(&self) -> usize {
        self.action_index
    }

    pub fn frame(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.path else { return };
        if !self.scale_configured {
            let scale = std::env::var("TERMINATOR_TEST_SCALE")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|s| (1.0..=2.0).contains(s))
                .unwrap_or(1.0);
            self.ticking
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let ticking = self.ticking.clone();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                while ticking.load(std::sync::atomic::Ordering::Relaxed) {
                    repaint.request_repaint();
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            ctx.set_pixels_per_point(scale);
            let background = std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some();
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if background {
                egui::WindowLevel::AlwaysOnBottom
            } else {
                egui::WindowLevel::AlwaysOnTop
            }));
            if background && std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_none() {
                // Synthetic fixtures must not intercept the user's real wheel or clicks.
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
            }
            self.scale_configured = true;
            if std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            eprintln!("Native fixture initialized at scale {scale}");
        }
        if self.scale_configured && !self.sized {
            let scale = std::env::var("TERMINATOR_TEST_SCALE")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(1.0);
            if (ctx.pixels_per_point() - scale).abs() < 0.01 {
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
        }
        if std::env::var_os("TERMINATOR_TEST_INPUT").is_some() {
            let ms = self.started.elapsed().as_millis();
            let event = match self.input_phase {
                0 if ms > 1200 => Some(egui::Event::Text("iUI_INSERT ".into())),
                1 if ms > 1500 => Some(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }),
                2 if ms > 1800 => Some(egui::Event::Text(":w\r".into())),
                _ => None,
            };
            if let Some(event) = event {
                ctx.input_mut(|i| {
                    i.focused = true;
                    i.events.push(event);
                });
                self.input_phase += 1;
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                eprintln!(
                    "Native fixture captured {}x{}",
                    image.width(),
                    image.height()
                );
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = image::save_buffer(
                    path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("Capture failed: {e}");
                }
                if std::env::var_os("TERMINATOR_TEST_KEEP_OPEN").is_none() {
                    if cfg!(target_os = "macos")
                        && std::env::var_os("TERMINATOR_TEST_NATIVE_QUIT").is_some()
                    {
                        crate::updater::fixture_native_quit();
                    } else {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    /// Called after UI layout, so a discarded sizing pass cannot supply a
    /// partial screenshot. Input injection remains at the start of the pass.
    pub fn capture(&mut self, ctx: &egui::Context) {
        if self.path.is_none() || ctx.will_discard() {
            return;
        }
        if !self.requested
            && self.started.elapsed()
                > Duration::from_millis(
                    std::env::var("TERMINATOR_CAPTURE_AFTER_MS")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(3000),
                )
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
            eprintln!("Native fixture requested capture");
        }
    }
}

#[derive(serde::Deserialize)]
struct FixtureAction {
    at_ms: u64,
    target: String,
    #[serde(default)]
    capture: bool,
    #[serde(default)]
    hover: bool,
    #[serde(default)]
    right_click: bool,
    #[serde(default)]
    scroll: Option<f32>,
    #[serde(default)]
    scroll_x: Option<f32>,
    #[serde(default)]
    wheel_unit: Option<String>,
    #[serde(default)]
    wheel_phase: Option<String>,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    key: Option<String>,
}
pub fn record(ctx: &egui::Context, name: &str, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(("fixture-target", name)), rect));
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.ticking
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_waits_for_the_final_layout_pass() {
        let ctx = egui::Context::default();
        let mut diagnostics = Diagnostics::default();
        diagnostics.path = Some("fixture.png".into());
        diagnostics.started = Instant::now() - Duration::from_secs(3600);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            if ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("fixture sizing pass");
                diagnostics.capture(ui.ctx());
                assert!(!diagnostics.requested);
            } else {
                diagnostics.capture(ui.ctx());
                assert!(diagnostics.requested);
            }
        });
        output.textures_delta.clear();
        assert!(diagnostics.requested);
    }
    #[test]
    fn desktop_typing_cannot_enter_an_isolated_fixture() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.path = Some("fixture.png".into());
        let mut input = egui::RawInput {
            events: vec![
                egui::Event::Text("unrelated desktop input".into()),
                egui::Event::Paste("clipboard".into()),
            ],
            ..Default::default()
        };
        diagnostics.input(&egui::Context::default(), &mut input);
        assert!(input.events.is_empty());
    }
}
