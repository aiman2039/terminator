//! Event-driven fixture actions. An action with `wait` fires when its target
//! holds still and its state conditions pass, not at its `at_ms` time.
use eframe::egui;
use std::{path::PathBuf, time::Duration};

/// Frames a target must keep the same rect before a waiting action fires.
const STABLE_FRAMES: u32 = 3;
/// No waiting action fires before this time after start. Startup panels
/// (Git status, explorer) can redraw a stable row with a new identity, and a
/// click before that is lost. Timed scripts started at about 1.1 s for the
/// same reason.
pub(super) const STARTUP_SETTLE: Duration = Duration::from_millis(1200);
/// How long an action may wait before the fixture log tells why.
const SLOW_WAIT: Duration = Duration::from_secs(5);

/// State conditions that must all pass on one frame before an action fires.
#[derive(serde::Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Wait {
    /// Targets that must be shown (and enabled, where recorded so).
    present: Vec<String>,
    /// Targets that must be gone, for example a closed popup.
    absent: Vec<String>,
    /// A file the driver or the app writes when a state is reached.
    file: Option<FileWait>,
    /// Minimum time after the previous action.
    settle_ms: u64,
    /// Frames the action's own target must hold still; default 3.
    stable_frames: Option<u32>,
    /// If this action's target is not shown after this long, fire the
    /// previous action again (at most `MAX_RETRIES` times per case). For
    /// steps that open a popup, where a lost click leaves nothing to wait on.
    retry_previous_ms: Option<u64>,
}

/// Retries of earlier actions allowed in one case.
const MAX_RETRIES: u32 = 3;

/// A file that must exist and, if given, match these checks.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWait {
    path: PathBuf,
    #[serde(default)]
    equals: Option<String>,
    #[serde(default)]
    contains: Option<String>,
    /// JSON pointer into the file; it must exist, and equal `value` if given.
    #[serde(default)]
    pointer: Option<String>,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

impl FileWait {
    fn holds(&self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return false;
        };
        self.equals.as_ref().is_none_or(|equals| text == *equals)
            && self
                .contains
                .as_ref()
                .is_none_or(|part| text.contains(part.as_str()))
            && self.pointer.as_ref().is_none_or(|pointer| {
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|json| json.pointer(pointer).cloned())
                    .is_some_and(|found| self.value.as_ref().is_none_or(|value| found == *value))
            })
    }
}

impl Wait {
    /// Why the action cannot fire yet, or `None` when every condition holds.
    /// `shown` tells whether a target is drawn; `stable` is how many frames the action's target held still; `None`
    /// for check actions, which have no target.
    pub(super) fn unmet(
        &self,
        shown: impl Fn(&str) -> bool,
        since_previous: Duration,
        stable: Option<u32>,
    ) -> Option<String> {
        if since_previous < Duration::from_millis(self.settle_ms) {
            return Some("settle time".into());
        }
        if let Some(frames) = stable
            && frames < self.stable_frames.unwrap_or(STABLE_FRAMES)
        {
            return Some("target to hold still".into());
        }
        if let Some(target) = self.present.iter().find(|target| !shown(target)) {
            return Some(format!("{target} to show"));
        }
        if let Some(target) = self.absent.iter().find(|target| shown(target)) {
            return Some(format!("{target} to close"));
        }
        if let Some(file) = &self.file
            && !file.holds()
        {
            return Some(format!("file {}", file.path.display()));
        }
        None
    }

    /// The retry delay, when this action may fire its previous one again.
    pub(super) fn retry_after(&self) -> Option<Duration> {
        self.retry_previous_ms.map(Duration::from_millis)
    }
}

/// The action that is waiting now: its target's last rect, how many frames
/// that rect held still, and when the wait began.
#[derive(Default)]
pub(super) struct Pending {
    index: Option<usize>,
    rect: Option<egui::Rect>,
    frames: u32,
    since: Duration,
    reported: bool,
    /// Retries used in this case; kept across index changes.
    retries: u32,
}

impl Pending {
    /// Records this frame's target rect for action `index` and returns how
    /// many consecutive frames it held still (0 while it is not shown).
    pub(super) fn observe(&mut self, index: usize, rect: Option<egui::Rect>, now: Duration) -> u32 {
        if self.index != Some(index) {
            *self = Self {
                index: Some(index),
                since: now,
                retries: self.retries,
                ..Self::default()
            };
        }
        self.frames = match rect {
            Some(_) if rect == self.rect => self.frames.saturating_add(1),
            Some(_) => 1,
            None => 0,
        };
        self.rect = rect;
        self.frames
    }

    /// Whether the waiting action should fire its previous action again:
    /// its target stayed hidden for `after`, and retries remain. Counts the
    /// retry and restarts the wait.
    pub(super) fn take_retry(&mut self, after: Duration, now: Duration) -> bool {
        if self.frames > 0 || self.retries >= MAX_RETRIES || now.saturating_sub(self.since) < after
        {
            return false;
        }
        self.retries = self.retries.saturating_add(1);
        self.index = None;
        true
    }

    /// Logs once why an action still waits after `SLOW_WAIT`.
    pub(super) fn report_slow(&mut self, target: &str, reason: &str, now: Duration) {
        if !self.reported && now.saturating_sub(self.since) >= SLOW_WAIT {
            self.reported = true;
            crate::console::Console::err(format_args!("Fixture waiting for {target}: {reason}"));
        }
    }
}
