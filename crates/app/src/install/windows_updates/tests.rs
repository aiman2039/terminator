use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn release(latest: Latest) -> FetchResult {
    Ok(Fetched::Latest { latest, etag: None })
}

fn failure(rate_limited: Option<Duration>) -> FetchResult {
    Err(FetchError {
        message: "boom".into(),
        rate_limited,
    })
}

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
        download_url("https://example.com/release", &assets, WINDOWS_ASSET_SUFFIX),
        "https://example.com/win.zip"
    );
    assert_eq!(
        download_url(
            "https://example.com/release",
            &assets,
            "-x86_64-unknown-linux-gnu.tar.gz"
        ),
        "https://example.com/linux.tgz"
    );
    assert_eq!(
        download_url("https://example.com/release", &[], WINDOWS_ASSET_SUFFIX),
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
    assert!(check.finish(release((
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
        format!("terminator-v9999.0.0{}", platform_asset_suffix()),
        "https://example.com/win.zip".into(),
    )];
    assert!(check.finish(release((
        "v9999.0.0".into(),
        "https://example.com/release".into(),
        assets
    ))));
    assert!(matches!(&check.status, Status::Available { version, url }
            if version == "v9999.0.0" && url == "https://example.com/win.zip"));
    // Same version again: no duplicate offer, first URL kept.
    assert!(check.finish(release((
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
    assert!(check.finish(release((
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
    assert!(check.finish(release((
        "v0.0.0".into(),
        "https://example.com/release".into(),
        Vec::new()
    ))));
    assert!(matches!(check.status, Status::Current));
    assert!(check.finish(failure(None)));
    assert!(matches!(check.status, Status::Failed(_)));
}

#[test]
fn failed_recheck_keeps_download_offer() {
    let mut check = WindowsUpdateCheck::new();
    assert!(check.finish(release((
        "v9999.0.0".into(),
        "https://example.com/release".into(),
        Vec::new()
    ))));
    assert!(check.finish(failure(None)));
    assert!(matches!(&check.status, Status::Available { version, .. }
            if version == "v9999.0.0"));
}

#[test]
fn not_modified_reuses_cached_release() {
    let mut check = WindowsUpdateCheck::new();
    assert!(check.finish(Ok(Fetched::Latest {
        latest: (
            "v0.0.0".into(),
            "https://example.com/release".into(),
            Vec::new()
        ),
        etag: Some("\"abc\"".into()),
    })));
    assert_eq!(check.etag.as_deref(), Some("\"abc\""));
    check.status = Status::Checking;
    assert!(check.finish(Ok(Fetched::NotModified)));
    assert!(matches!(check.status, Status::Current));
}

#[test]
fn rate_limit_backs_off_beyond_one_minute() {
    let mut check = WindowsUpdateCheck::new();
    assert!(check.finish(failure(Some(Duration::ZERO))));
    let first = check.next;
    assert!(first >= Duration::from_mins(1));
    assert!(check.finish(failure(Some(Duration::ZERO))));
    assert!(check.next >= first.saturating_add(Duration::from_mins(1)));
    // The server's own wait wins over the doubling backoff.
    assert_eq!(
        rate_limit_delay(0, Duration::from_mins(10)),
        Duration::from_mins(10)
    );
    assert_eq!(rate_limit_delay(30, Duration::ZERO), MAX_BACKOFF);
    // A good reply resets the backoff.
    assert!(check.finish(release((
        "v0.0.0".into(),
        "https://example.com/release".into(),
        Vec::new()
    ))));
    assert_eq!(check.rate_limit_streak, 0);
}

fn headers(pairs: &[(&'static str, &'static str)]) -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    for (name, value) in pairs {
        map.insert(*name, reqwest::header::HeaderValue::from_static(value));
    }
    map
}

#[test]
fn rate_limit_replies_are_told_apart_from_other_failures() {
    let now = UNIX_EPOCH.checked_add(Duration::from_secs(1_000)).unwrap();
    let wait = |status, pairs: &[(&'static str, &'static str)]| {
        feed::rate_limit_wait(status, &headers(pairs), now)
    };
    assert_eq!(wait(200, &[]), None);
    assert_eq!(wait(403, &[("x-ratelimit-remaining", "12")]), None);
    assert_eq!(
        wait(403, &[("retry-after", "30")]),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        wait(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1300")
            ]
        ),
        Some(Duration::from_mins(5))
    );
    assert_eq!(
        feed::rate_limit_wait(429, &headers(&[]), SystemTime::now()),
        Some(Duration::ZERO)
    );
}
