//! Windows release polling: a notify-only replacement for Sparkle.
//!
//! There is no signed auto-installer on Windows yet. Once a minute (and on
//! demand from Settings → Updates) the GUI asks the GitHub Releases API for
//! the latest tag; a newer tag surfaces a download link for the published
//! `terminator-*-x86_64-pc-windows-msvc.zip`. Nothing downloads or installs
//! automatically, and development/isolated launches never touch the feed.

use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

/// Release feed owner, matching the published release links in docs.
pub const FEED_REPO: &str = "aiman2039/terminator";
/// Windows package suffix, matching `cargo xtask package` output.
pub const WINDOWS_ASSET_SUFFIX: &str = "-x86_64-pc-windows-msvc.zip";

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

/// Prefer the Windows zip asset; fall back to the release page itself.
#[must_use]
pub fn download_url(html_url: &str, assets: &[(String, String)]) -> String {
    assets
        .iter()
        .find(|(name, _)| name.ends_with(WINDOWS_ASSET_SUFFIX))
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

/// Latest-release lookup: `(tag, page URL, [(asset name, download URL)])`.
type FetchResult = Result<(String, String, Vec<(String, String)>), String>;

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
        }
    }

    /// Fold a finished worker result into status. Returns true when the UI
    /// should repaint soon.
    fn finish(&mut self, result: FetchResult) -> bool {
        self.worker = None;
        self.probing = false;
        // The in-flight probe satisfies a click made while probing.
        self.manual = false;
        match result {
            Ok((tag, html_url, assets)) => {
                if newer_than(&tag) {
                    let url = download_url(&html_url, &assets);
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
                self.status = Status::Failed(error);
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
        if let Some(worker) = &self.worker
            && let Ok(result) = worker.try_recv()
        {
            return self.finish(result);
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
            .checked_add(Duration::from_mins(1))
            .unwrap_or(Duration::MAX);
        self.manual = false;
        self.probing = true;
        self.status = Status::Checking;
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(fetch_latest());
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
                    ui.hyperlink_to("Download the Windows release", url);
                    ui.label(
                        "Unzip over your install and restart. Sessions stay in the background.",
                    );
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

fn fetch_latest() -> FetchResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("update runtime: {e}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .user_agent(format!("terminator/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("update client: {e}"))?;
        let body = client
            .get(format!(
                "https://api.github.com/repos/{FEED_REPO}/releases/latest"
            ))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("update request: {e}"))?
            .error_for_status()
            .map_err(|e| format!("update feed: {e}"))?
            .text()
            .await
            .map_err(|e| format!("update feed: {e}"))?;
        let response: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("update feed: {e}"))?;
        let tag = response
            .get("tag_name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "update feed has no tag".to_string())?
            .to_owned();
        let html_url = response
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("https://github.com/aiman2039/terminator/releases")
            .to_owned();
        let mut assets = Vec::new();
        if let Some(list) = response.get("assets").and_then(serde_json::Value::as_array) {
            for asset in list {
                if let (Some(name), Some(url)) = (
                    asset.get("name").and_then(serde_json::Value::as_str),
                    asset
                        .get("browser_download_url")
                        .and_then(serde_json::Value::as_str),
                ) {
                    assets.push((name.to_owned(), url.to_owned()));
                }
            }
        }
        Ok((tag, html_url, assets))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_tags_win_and_garbage_is_ignored() {
        let current = env!("CARGO_PKG_VERSION");
        assert!(!newer_than(current));
        assert!(!newer_than(&format!("v{current}-rc.1")));
        assert!(!newer_than("not-a-version"));
        assert!(!newer_than("1.2"));
        assert!(!newer_than("v1.2.3.4"));
        // A far-future tag is always newer than any real build.
        assert!(newer_than("v9999.0.0"));
        assert!(!newer_than("v0.0.0"));
    }

    #[test]
    fn prerelease_tags_lose_to_their_own_release() {
        assert!(parse_tag("v1.2.3-rc.1").unwrap().1);
        assert!(!parse_tag("v1.2.3").unwrap().1);
        assert!(parse_tag("v1.2").is_none());
    }

    #[test]
    fn windows_asset_wins_with_release_page_fallback() {
        let assets = vec![
            (
                "terminator-v9.9.9-x86_64-pc-windows-msvc.zip".into(),
                "https://example.com/win.zip".into(),
            ),
            (
                "terminator-v9.9.9-x86_64-unknown-linux-gnu.tar.gz".into(),
                "https://example.com/linux.tgz".into(),
            ),
        ];
        assert_eq!(
            download_url("https://example.com/release", &assets),
            "https://example.com/win.zip"
        );
        assert_eq!(
            download_url("https://example.com/release", &[]),
            "https://example.com/release"
        );
    }

    #[test]
    fn disabled_checks_stay_idle() {
        let mut check = WindowsUpdateCheck::new();
        assert!(!check.poll(false));
        // Disabled checks never probe.
        assert!(matches!(check.status, Status::Idle));
    }

    #[test]
    fn manual_check_runs_once_while_automatic_checks_are_off() {
        // `eligible` stands in for `feed_eligible()`, which is always false
        // in development builds; the worker thread it spawns is fire-and-
        // forget and never blocks these assertions.
        let mut check = WindowsUpdateCheck::new();
        assert!(!check.poll_with(false, true));
        assert!(matches!(check.status, Status::Idle));
        // The Settings button requests a one-shot check.
        check.check_now();
        assert!(!check.poll_with(false, true));
        assert!(matches!(check.status, Status::Checking));
        assert!(check.finish(Ok((
            "v9999.0.0".into(),
            "https://example.com/release".into(),
            Vec::new()
        ))));
        assert!(matches!(&check.status, Status::Available { version, .. }
                if version == "v9999.0.0"));
        // One-shot: later polls stay quiet without re-enabling automation.
        assert!(!check.poll_with(false, true));
        assert!(matches!(&check.status, Status::Available { .. }));
    }

    #[test]
    fn newer_release_is_offered_once() {
        let mut check = WindowsUpdateCheck::new();
        let assets = vec![(
            "terminator-v9999.0.0-x86_64-pc-windows-msvc.zip".into(),
            "https://example.com/win.zip".into(),
        )];
        assert!(check.finish(Ok((
            "v9999.0.0".into(),
            "https://example.com/release".into(),
            assets
        ))));
        assert!(matches!(&check.status, Status::Available { version, url }
                if version == "v9999.0.0" && url == "https://example.com/win.zip"));
        // Same version again: no duplicate offer, first URL kept.
        assert!(check.finish(Ok((
            "v9999.0.0".into(),
            "https://example.com/release".into(),
            Vec::new()
        ))));
        assert!(matches!(&check.status, Status::Available { version, url }
                if version == "v9999.0.0" && url == "https://example.com/win.zip"));
        assert_eq!(check.offered.len(), 1);
        // A repeat sighting after a new poll parks at Checking while probing,
        // then restores the download offer when the check completes.
        check.status = Status::Checking;
        assert!(check.finish(Ok((
            "v9999.0.0".into(),
            "https://example.com/release".into(),
            Vec::new()
        ))));
        assert!(matches!(&check.status, Status::Available { version, .. }
                if version == "v9999.0.0"));
    }

    #[test]
    fn older_release_marks_current_and_errors_mark_failed() {
        let mut check = WindowsUpdateCheck::new();
        assert!(check.finish(Ok((
            "v0.0.0".into(),
            "https://example.com/release".into(),
            Vec::new()
        ))));
        assert!(matches!(check.status, Status::Current));
        assert!(check.finish(Err("boom".into())));
        assert!(matches!(check.status, Status::Failed(_)));
    }
}
