//! GitHub Releases request for the notify-only update check.
//!
//! The request honors `HTTP(S)_PROXY`, sends the cached `ETag` so unchanged
//! releases answer `304 Not Modified` (which GitHub does not count against
//! the unauthenticated rate limit), and reports rate-limit replies with the
//! wait the server asks for.

use super::FEED_REPO;
use reqwest::header::HeaderMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Latest release: `(tag, page URL, [(asset name, download URL)], notes)`.
/// Notes are the release `body` (Markdown), empty when the feed has none.
pub(super) type Latest = (String, String, Vec<(String, String)>, String);

#[derive(Debug)]
pub(super) enum Fetched {
    Latest {
        latest: Latest,
        etag: Option<String>,
    },
    NotModified,
}

#[derive(Debug)]
pub(super) struct FetchError {
    pub message: String,
    /// Set for 403/429 rate-limit replies: the server's requested wait,
    /// or zero when it gave none.
    pub rate_limited: Option<Duration>,
}

impl FetchError {
    pub(super) fn other(message: String) -> Self {
        Self {
            message,
            rate_limited: None,
        }
    }
}

pub(super) type FetchResult = Result<Fetched, FetchError>;

pub(super) fn fetch_latest(etag: Option<String>) -> FetchResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| FetchError::other(format!("update runtime: {e}")))?;
    runtime.block_on(request(etag))
}

async fn request(etag: Option<String>) -> FetchResult {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("terminator/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| FetchError::other(format!("update client: {e}")))?;
    let mut request = client
        .get(format!(
            "https://api.github.com/repos/{FEED_REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json");
    if let Some(etag) = &etag {
        request = request.header("If-None-Match", etag);
    }
    let response = request
        .send()
        .await
        .map_err(|e| FetchError::other(format!("update request: {e}")))?;
    let status = response.status().as_u16();
    if status == 304 {
        return Ok(Fetched::NotModified);
    }
    if let Some(wait) = rate_limit_wait(status, response.headers(), SystemTime::now()) {
        return Err(FetchError {
            message: "GitHub rate limit reached; retrying later".into(),
            rate_limited: Some(wait),
        });
    }
    let new_etag = header(response.headers(), "etag").map(str::to_owned);
    let body = response
        .error_for_status()
        .map_err(|e| FetchError::other(format!("update feed: {e}")))?
        .text()
        .await
        .map_err(|e| FetchError::other(format!("update feed: {e}")))?;
    let latest = parse_latest(&body).map_err(FetchError::other)?;
    Ok(Fetched::Latest {
        latest,
        etag: new_etag,
    })
}

/// The wait a 403/429 reply asks for, or `None` when the reply is not a
/// rate limit. A 403 without rate-limit headers is an ordinary failure.
pub(super) fn rate_limit_wait(
    status: u16,
    headers: &HeaderMap,
    now: SystemTime,
) -> Option<Duration> {
    if status != 403 && status != 429 {
        return None;
    }
    let retry_after = header(headers, "retry-after");
    let remaining = header(headers, "x-ratelimit-remaining");
    let reset = header(headers, "x-ratelimit-reset");
    if let Some(secs) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return Some(Duration::from_secs(secs));
    }
    if remaining.map(str::trim) == Some("0") {
        let now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let reset = reset.and_then(|v| v.trim().parse::<u64>().ok());
        return Some(Duration::from_secs(
            reset.map_or(0, |reset| reset.saturating_sub(now)),
        ));
    }
    (status == 429).then_some(Duration::ZERO)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

pub(super) fn parse_latest(body: &str) -> Result<Latest, String> {
    let response: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("update feed: {e}"))?;
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
    let notes = response
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok((tag, html_url, assets, notes))
}
