//! Opt-in renderer capture for native integration tests. Never enabled in normal builds.
mod frame;
mod input;
#[cfg(test)]
mod tests;

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
    last_action: Option<(u64, Duration)>,
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
            last_action: None,
            release: None,
            held: None,
            pointer: None,
            ticking: std::sync::Arc::default(),
        }
    }
}
impl Diagnostics {
    fn action_ready(&self, action: &FixtureAction, elapsed: Duration) -> bool {
        let scheduled = Duration::from_millis(action.at_ms);
        if elapsed < scheduled {
            return false;
        }
        // A late render must not compress the script's settling intervals.
        // Popups and view changes still need their planned time after the
        // previous action actually ran, even if all timestamps are overdue.
        self.last_action.is_none_or(|(previous_ms, actual)| {
            elapsed.saturating_sub(actual)
                >= Duration::from_millis(action.at_ms.saturating_sub(previous_ms))
        })
    }

    pub fn actions_completed(&self) -> usize {
        self.action_index
    }

    fn request_fixture_capture(&self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let callback_ctx = ctx.clone();
        ctx.request_screenshot(move |image| {
            crate::console::Console::err(format_args!(
                "Native fixture captured {}x{}",
                image.width(),
                image.height()
            ));
            let bytes = image
                .pixels
                .iter()
                .flat_map(eframe::egui::Color32::to_array)
                .collect::<Vec<_>>();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = image::save_buffer(
                &path,
                &bytes,
                u32::try_from(image.width()).unwrap_or(u32::MAX),
                u32::try_from(image.height()).unwrap_or(u32::MAX),
                image::ColorType::Rgba8,
            ) {
                crate::console::Console::err(format_args!("Capture failed: {e}"));
            }
            if std::env::var_os("TERMINATOR_TEST_KEEP_OPEN").is_none() {
                if cfg!(target_os = "macos")
                    && std::env::var_os("TERMINATOR_TEST_NATIVE_QUIT").is_some()
                {
                    crate::updater::fixture_native_quit();
                } else {
                    callback_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        });
    }

    /// Called after UI layout, so a discarded sizing pass cannot supply a
    /// partial screenshot. Input injection remains at the start of the pass.
    pub fn capture(&mut self, ctx: &egui::Context) {
        if self.path.is_none() || ctx.will_discard() {
            return;
        }
        if std::env::var_os("TERMINATOR_TEST_CAPTURE_COPY").is_some() {
            // Fixtures inspect the native clipboard command without replacing
            // the user's system clipboard with public sample text.
            ctx.output_mut(|output| {
                output.commands.retain(|command| {
                    if let egui::OutputCommand::CopyText(text) = command {
                        crate::console::Console::err(format_args!("Fixture copied text: {text:?}"));
                        false
                    } else {
                        true
                    }
                });
            });
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
        if !self.requested
            && self.started.elapsed() > after
            // A scripted action must never lose its target to the capture
            // deadline: parallel fixtures on a loaded machine paint late, and
            // a skipped action reads as a missing step. Wait for the pending
            // actions; a target that never appears still fails, because the
            // action is never logged and the driver kills the window.
            && self.action_index >= self.actions.len()
        {
            self.request_fixture_capture(ctx);
            self.requested = true;
            crate::console::Console::err(format_args!("Native fixture requested capture"));
        } else if self.requested
            && !self.force_closed
            // Native-input drivers own their lifetime and enforce their own
            // deadlines. A completed screenshot must not close their window.
            && std::env::var_os("TERMINATOR_TEST_KEEP_OPEN").is_none()
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
            crate::console::Console::err(format_args!(
                "Native fixture forced close without capture"
            ));
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
    ctrl: bool,
    #[serde(default)]
    command: bool,
    #[serde(default)]
    copy: bool,
    #[serde(default)]
    modifier: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    key: Option<String>,
}
pub fn record(ctx: &egui::Context, name: &str, rect: egui::Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::unique(("fixture-target", name)), rect);
        data.insert_temp(egui::Id::unique(("fixture-target-frame", name)), frame);
    });
}

/// Popups place their contents on an invisible, non-interactable sizing pass
/// the first time they open. A scripted click on a rect from that pass lands
/// where nothing can receive it, so popup rows use this variant and stay
/// unavailable until the first real layout pass.
pub fn record_ui(ui: &egui::Ui, name: &str, rect: egui::Rect) {
    if ui.is_sizing_pass() {
        return;
    }
    record(ui.ctx(), name, rect);
}

pub fn current_rect(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    let current = ctx.cumulative_frame_nr();
    ctx.data(|data| {
        let frame = data.get_temp::<u64>(egui::Id::unique(("fixture-target-frame", name)))?;
        (frame == current || frame.checked_add(1) == Some(current))
            .then(|| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
            .flatten()
    })
}

pub fn record_response(ctx: &egui::Context, name: &str, response: &egui::Response) {
    if response.enabled() {
        record(ctx, name, response.rect);
    } else {
        ctx.data_mut(|data| {
            data.remove::<egui::Rect>(egui::Id::unique(("fixture-target", name)));
        });
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.ticking
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
