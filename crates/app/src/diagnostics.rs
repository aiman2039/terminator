//! Opt-in renderer capture for native integration tests. Never enabled in normal builds.
use eframe::egui;
use std::time::{Duration, Instant};

/// How long after the capture deadline to wait for a screenshot event
/// before closing anyway. Occluded windows never deliver screenshots.
const GRACE: Duration = Duration::from_secs(10);
pub struct Diagnostics {
    started: Instant,
    requested: bool,
    force_closed: bool,
    scale_configured: bool,
    sized: bool,
    input_phase: u8,
    path: Option<std::path::PathBuf>,
    actions: Vec<FixtureAction>,
    action_index: usize,
    release: Option<(egui::Pos2, egui::PointerButton)>,
    held: Option<egui::PointerButton>,
    pointer: Option<egui::Pos2>,
    ticking: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            requested: false,
            force_closed: false,
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
            held: None,
            pointer: None,
            ticking: std::sync::Arc::default(),
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
                modifiers: egui::Modifiers::default(),
            });
        } else if let Some(action) = self.actions.get(self.action_index)
            && self.started.elapsed().as_millis() >= u128::from(action.at_ms)
            && let Some(rect) = ctx.data(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", &action.target)))
            })
        {
            let pos = if let Some([x, y]) = action.hover_offset {
                egui::pos2(rect.min.x + x, rect.min.y + y)
            } else if action.hover {
                egui::pos2(rect.min.x + 45.0, rect.min.y + 10.0)
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
                        modifiers: egui::Modifiers::default(),
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
                if action.release_button {
                    // Ends a held drag (see `hold`): the press stays down
                    // across the hover moves between the two actions.
                    if let Some(held) = self.held.take() {
                        input.events.push(egui::Event::PointerButton {
                            pos,
                            button: held,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                } else {
                    input.events.push(egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    });
                    if action.hold {
                        self.held = Some(button);
                    } else {
                        self.release = Some((pos, button));
                    }
                }
            }
            if action.capture {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            eprintln!("Fixture action: {}", action.target);
            self.action_index = self.action_index.saturating_add(1);
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
                    .flat_map(eframe::egui::Color32::to_array)
                    .collect::<Vec<_>>();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = image::save_buffer(
                    path,
                    &bytes,
                    u32::try_from(image.width()).unwrap_or(u32::MAX),
                    u32::try_from(image.height()).unwrap_or(u32::MAX),
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
    /// Called after UI layout, so a discarded sizing pass cannot supply a
    /// partial screenshot. Input injection remains at the start of the pass.
    pub fn capture(&mut self, ctx: &egui::Context) {
        if self.path.is_none() || ctx.will_discard() {
            return;
        }
        // Driver-signaled graceful exit: the file's presence closes the
        // fixture once outside observation is complete. Inert unless the
        // env var names a path, and unlike screenshots it does not need a
        // rendered frame, so it also works while occluded.
        if std::env::var_os("TERMINATOR_TEST_EXIT_MARKER")
            .is_some_and(|path| std::path::Path::new(&path).exists())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let after = Duration::from_millis(
            std::env::var("TERMINATOR_CAPTURE_AFTER_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
        );
        if !self.requested && self.started.elapsed() > after {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.requested = true;
            eprintln!("Native fixture requested capture");
        } else if self.requested
            && !self.force_closed
            && after
                .checked_add(GRACE)
                .is_some_and(|deadline| self.started.elapsed() > deadline)
        {
            // An occluded window never delivers the screenshot, so the
            // normal screenshot-then-close exit never fires. Close anyway
            // once the deadline is well past; step captures taken while
            // visible already wrote the artifact.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.force_closed = true;
            eprintln!("Native fixture forced close without capture");
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
    hover_offset: Option<[f32; 2]>,
    #[serde(default)]
    right_click: bool,
    /// Keep the press down across later hover moves (pane drags).
    #[serde(default)]
    hold: bool,
    /// Release a held press at this action's position.
    #[serde(default)]
    release_button: bool,
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
        diagnostics.started = Instant::now()
            .checked_sub(Duration::from_secs(4))
            .unwrap_or_else(Instant::now);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
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
