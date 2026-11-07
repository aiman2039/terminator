//! Windows and Linux release polling: a notify-only replacement for Sparkle.
//!
//! There is no signed auto-installer outside macOS yet. Once a minute (and on
//! demand from Settings → Updates, the header menu or the palette) the GUI
//! asks the GitHub Releases API for the latest tag; a newer tag surfaces a
//! download link for this platform's published archive. Nothing downloads or
//! installs automatically, and development/isolated launches never touch the
//! feed.

mod feed;

use crate::*;
use feed::{FetchError, FetchResult, Fetched, Latest};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

/// Release feed owner, matching the published release links in docs.
pub const FEED_REPO: &str = "aiman2039/terminator";
/// Windows package suffix, matching `cargo xtask package` output.
pub const WINDOWS_ASSET_SUFFIX: &str = "-x86_64-pc-windows-msvc.zip";

/// Release asset suffix for this build's platform.
fn platform_asset_suffix() -> String {
    if cfg!(windows) {
        WINDOWS_ASSET_SUFFIX.into()
    } else {
        format!("-{}-unknown-linux-gnu.tar.gz", std::env::consts::ARCH)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Release {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Parse a `vMAJOR.MINOR.PATCH` tag. Pre-release suffixes compare as older
/// than the release with the same numbers.
fn parse_tag(tag: &str) -> Option<(Release, bool)> {
    let tag = tag.strip_prefix('v')?;
    let (numbers, pre) = match tag.split_once('-') {
        Some((numbers, _)) => (numbers, true),
        None => (tag, false),
    };
    let mut parts = numbers.split('.');
    let release = Release {
        major: parts.next()?.parse().ok()?,
        minor: parts.next()?.parse().ok()?,
        patch: parts.next()?.parse().ok()?,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((release, pre))
}

fn current() -> Release {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    Release {
        major: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        minor: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        patch: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
    }
}

/// The release tag is newer than this build. Pre-releases never win against
/// the same numbers.
#[must_use]
pub fn newer_than(tag: &str) -> bool {
    let Some((release, pre)) = parse_tag(tag) else {
        return false;
    };
    let current = current();
    (release.major, release.minor, release.patch) > (current.major, current.minor, current.patch)
        || ((release.major, release.minor, release.patch)
            == (current.major, current.minor, current.patch)
            && !pre
            && is_prerelease_build())
}

fn is_prerelease_build() -> bool {
    env!("CARGO_PKG_VERSION").contains('-')
}

/// Prefer the asset with `suffix`; fall back to the release page itself.
#[must_use]
pub fn download_url(html_url: &str, assets: &[(String, String)], suffix: &str) -> String {
    assets
        .iter()
        .find(|(name, _)| name.ends_with(suffix))
        .map(|(_, url)| url.clone())
        .unwrap_or_else(|| html_url.into())
}

/// Development and isolated launches never touch the production feed.
#[must_use]
pub fn feed_eligible() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    std::env::var_os("TERMINATOR_DATA_DIR").is_none()
        && !std::env::args().any(|a| a == "--data-dir")
        && std::env::var_os("TERMINATOR_CONFIG_DIR").is_none()
        && std::env::var_os("TERMINATOR_RUNTIME_DIR").is_none()
}

#[derive(Debug, PartialEq)]
enum Status {
    Idle,
    Checking,
    Current,
    Available { version: String, url: String },
    Failed(String),
}

/// Normal poll cadence (AGENTS.md: once per minute).
const POLL_INTERVAL: Duration = Duration::from_mins(1);
/// Longest wait after repeated rate-limit replies.
const MAX_BACKOFF: Duration = Duration::from_hours(1);

/// Wait before the next poll after `streak` rate-limit replies in a row:
/// the server's requested wait or doubling minutes, whichever is longer.
fn rate_limit_delay(streak: u32, server_wait: Duration) -> Duration {
    let doubling = POLL_INTERVAL.saturating_mul(1u32.checked_shl(streak.min(6)).unwrap_or(64));
    doubling.max(server_wait).min(MAX_BACKOFF)
}

pub struct WindowsUpdateCheck {
    started: Instant,
    next: Duration,
    probing: bool,
    /// One-shot Settings-button request: runs once even when automatic
    /// checks are off, without re-enabling them.
    manual: bool,
    status: Status,
    offered: std::collections::HashSet<String>,
    worker: Option<mpsc::Receiver<FetchResult>>,
    /// Last release body and its `ETag`, reused on `304 Not Modified`.
    etag: Option<String>,
    cached: Option<Latest>,
    /// Rate-limit replies in a row; drives the poll backoff.
    rate_limit_streak: u32,
}

impl WindowsUpdateCheck {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            next: Duration::ZERO,
            probing: false,
            manual: false,
            status: Status::Idle,
            offered: std::collections::HashSet::new(),
            worker: None,
            etag: None,
            cached: None,
            rate_limit_streak: 0,
        }
    }

    /// Fold a finished worker result into status. Returns true when the UI
    /// should repaint soon.
    fn finish(&mut self, result: FetchResult) -> bool {
        self.worker = None;
        self.probing = false;
        // The in-flight probe satisfies a click made while probing.
        self.manual = false;
        let result = match result {
            Ok(Fetched::Latest { latest, etag }) => {
                self.etag = etag;
                self.cached = Some(latest.clone());
                Ok(latest)
            }
            Ok(Fetched::NotModified) => self.cached.clone().ok_or_else(|| FetchError {
                message: "update feed: not modified without a cached release".into(),
                rate_limited: None,
            }),
            Err(error) => Err(error),
        };
        match result {
            Ok((tag, html_url, assets)) => {
                self.rate_limit_streak = 0;
                if newer_than(&tag) {
                    let url = download_url(&html_url, &assets, &platform_asset_suffix());
                    // One offer per version per launch, like Sparkle.
                    let first = self.offered.insert(tag.clone());
                    // A repeat sighting must still restore the download offer:
                    // polling parks the status at Checking while probing.
                    if first || !matches!(self.status, Status::Available { .. }) {
                        self.status = Status::Available { version: tag, url };
                    }
                } else {
                    self.status = Status::Current;
                }
            }
            Err(error) => {
                if let Some(wait) = error.rate_limited {
                    let delay = rate_limit_delay(self.rate_limit_streak, wait);
                    self.rate_limit_streak = self.rate_limit_streak.saturating_add(1);
                    self.next = self.started.elapsed().saturating_add(delay);
                }
                // A transient failure must not hide a known download offer.
                if !matches!(self.status, Status::Available { .. }) {
                    self.status = Status::Failed(error.message);
                }
            }
        }
        true
    }

    /// Minute-cadence poll for the GUI update loop. Returns true when the UI
    /// should repaint soon (a check finished).
    pub fn poll(&mut self, enabled: bool) -> bool {
        self.poll_with(enabled, feed_eligible())
    }

    fn poll_with(&mut self, enabled: bool, eligible: bool) -> bool {
        if let Some(worker) = &self.worker {
            match worker.try_recv() {
                Ok(result) => return self.finish(result),
                // The check thread ended without a result (it panicked).
                Err(mpsc::TryRecvError::Disconnected) => {
                    let stopped =
                        FetchError::other("update check stopped before it finished".into());
                    return self.finish(Err(stopped));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !eligible || self.probing {
            return false;
        }
        if !enabled && !self.manual {
            return false;
        }
        if self.started.elapsed() < self.next {
            return false;
        }
        self.next = self
            .started
            .elapsed()
            .checked_add(POLL_INTERVAL)
            .unwrap_or(Duration::MAX);
        self.manual = false;
        self.probing = true;
        // Keep a known download offer visible while re-checking.
        if !matches!(self.status, Status::Available { .. }) {
            self.status = Status::Checking;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        let etag = self.etag.clone();
        std::thread::spawn(move || {
            let _ = tx.send(feed::fetch_latest(etag));
        });
        false
    }

    /// Immediate re-check for the Settings button. Runs once on the next
    /// poll even when automatic checks are off.
    pub fn check_now(&mut self) {
        self.next = Duration::ZERO;
        self.started = Instant::now();
        self.manual = true;
    }

    #[cfg(test)]
    pub fn manual_check_pending(&self) -> bool {
        self.manual
    }

    pub fn settings(&mut self, ui: &mut egui::Ui, enabled: &mut bool) {
        ui.heading("Updates");
        if !feed_eligible() {
            ui.label("This development build does not check for updates. Install a published Terminator release.");
        } else {
            match &self.status {
                Status::Idle | Status::Current => {
                    ui.label("Terminator is up to date.");
                }
                Status::Checking => {
                    ui.label("Checking for updates…");
                }
                Status::Available { version, url } => {
                    ui.label(format!("Terminator {version} is available."));
                    if cfg!(windows) {
                        ui.hyperlink_to("Download the Windows release", url);
                        ui.label(
                            "Unzip over your install and restart. Sessions stay in the background.",
                        );
                    } else {
                        ui.hyperlink_to("Download the Linux release", url);
                        ui.label(
                            "Extract over your install and restart. Sessions stay in the background.",
                        );
                    }
                }
                Status::Failed(error) => {
                    ui.label(format!("The update check failed: {error}"));
                }
            }
            if ui
                .checkbox(enabled, "Check for updates every minute")
                .changed()
            {
                self.next = Duration::ZERO;
                self.started = Instant::now();
            }
            if ui.button("Check for Updates…").clicked() {
                self.check_now();
            }
        }
        ui.hyperlink_to(
            "GitHub release notes",
            "https://github.com/aiman2039/terminator/releases",
        );
    }
}

impl Default for WindowsUpdateCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
