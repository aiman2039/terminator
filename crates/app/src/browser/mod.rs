//! GUI-owned browser tab identity. The native view is mounted later.
pub(crate) mod extras;
pub(crate) mod favicon;
#[cfg(feature = "webview")]
pub(crate) mod host;
#[cfg(not(feature = "webview"))]
#[path = "host_off.rs"]
pub(crate) mod host;
pub(crate) mod nav;
#[cfg(feature = "webview")]
pub(crate) mod view;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use terminator_core::metadata;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) enum BrowserTarget {
    File(PathBuf),
    Url(String),
}

impl BrowserTarget {
    pub fn title(&self) -> String {
        match self {
            Self::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Url(url) => url.clone(),
        }
    }

    pub fn file(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Url(_) => None,
        }
    }

    pub fn from_http_url(value: &str) -> Result<Self> {
        Ok(Self::Url(parse_url(value)?))
    }
}

/// Address bar input: an http(s) URL, or a bare host such as `example.com`
/// (https) or `localhost:3000` (http). Other text searches with `search`
/// (an https template, see [`extras::search_url`]). Empty text and URLs
/// with another scheme are an error to show.
pub(crate) fn address_target(input: &str, search: &str) -> Result<BrowserTarget, &'static str> {
    const ERROR: &str = "Enter a web address, such as example.com or https://example.com.";
    let input = input.trim();
    if let Ok(target) = BrowserTarget::from_http_url(input) {
        return Ok(target);
    }
    if let Some(target) = host_target(input) {
        return Ok(target);
    }
    // `mailto:x` or `javascript:x` has a scheme: never a search.
    let other_scheme = !input.contains(char::is_whitespace) && url::Url::parse(input).is_ok();
    if input.is_empty() || other_scheme || input.contains("://") {
        return Err(ERROR);
    }
    extras::search_target(search, input).ok_or(ERROR)
}

fn host_target(input: &str) -> Option<BrowserTarget> {
    let host = input.split(['/', '?', '#']).next().unwrap_or_default();
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    let local = matches!(name, "localhost" | "127.0.0.1");
    let host_like = !input.contains("://")
        && !input.contains(char::is_whitespace)
        && (local || (name.contains('.') && !name.starts_with('.') && !name.ends_with('.')));
    let scheme = if local { "http" } else { "https" };
    host_like
        .then(|| BrowserTarget::from_http_url(&format!("{scheme}://{input}")).ok())
        .flatten()
}

pub(crate) fn supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            )
        })
}

/// egui layers drawn above panes. The native view draws over egui, so a
/// pane under one of these hides while it is open.
pub(crate) fn covers_webview(order: eframe::egui::Order) -> bool {
    use eframe::egui::Order;
    matches!(order, Order::Middle | Order::Foreground | Order::Tooltip)
}

pub(crate) fn parse_url(value: &str) -> Result<String> {
    Ok(metadata::http_url(value)?)
}

pub(crate) fn profile_dir(data: &Path) -> PathBuf {
    data.join("webview")
}

pub(crate) fn ensure_profile(data: &Path) -> Result<PathBuf> {
    let dir = profile_dir(data);
    std::fs::create_dir_all(&dir).context("Create isolated webview profile")?;
    Ok(dir)
}

pub(crate) fn href(target: &BrowserTarget) -> Result<String> {
    match target {
        BrowserTarget::File(path) => url::Url::from_file_path(path)
            .map(|url| url.to_string())
            .map_err(|()| anyhow::anyhow!("HTML path is not absolute")),
        BrowserTarget::Url(url) => parse_url(url),
    }
}

/// Local files a File tab's view may show: the opened file, and other HTML
/// files under its directory when reached from a local page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LocalScope {
    file: Option<PathBuf>,
}

impl LocalScope {
    pub fn of(target: &BrowserTarget) -> Self {
        Self {
            file: target.file().map(Path::to_path_buf),
        }
    }
}

/// Apply the same policy to initial loads, links, redirects, and history
/// navigation. `current` is the view's committed URL: a remote page may
/// return only to the opened file, never reach other local files.
pub(crate) fn navigation_target(
    value: &str,
    current: Option<&str>,
    scope: &LocalScope,
) -> Option<BrowserTarget> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "file" {
        return BrowserTarget::from_http_url(value).ok();
    }
    let opened = scope.file.as_deref()?;
    let path = url.to_file_path().ok()?;
    let from_local = current
        .and_then(|current| url::Url::parse(current).ok())
        .is_some_and(|current| current.scheme() == "file");
    let in_scope = from_local && opened.parent().is_some_and(|dir| path.starts_with(dir));
    (supported_file(&path) && (path == opened || in_scope)).then_some(BrowserTarget::File(path))
}

/// Target of a URL the view already shows; it passed the navigation check.
pub(crate) fn committed_target(value: &str, scope: &LocalScope) -> Option<BrowserTarget> {
    navigation_target(value, Some(value), scope)
}

/// A random persistent identifier belongs to this data directory, not the app bundle.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn profile_identifier(data: &Path) -> Result<[u8; 16]> {
    use fs2::FileExt;
    use std::io::{Read, Write};
    let path = ensure_profile(data)?.join("store-id");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        let id = uuid::Uuid::new_v4();
        file.write_all(id.as_bytes())?;
        file.sync_all()?;
        return Ok(*id.as_bytes());
    }
    Ok(*uuid::Uuid::from_slice(&bytes)
        .context("Invalid browser profile identifier")?
        .as_bytes())
}

/// Rewrite persisted `Tab::Html { path }` objects into `Tab::Browser`.
pub(crate) fn rewrite_html_tabs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(html) = map.remove("Html") {
                if let Some(path) = html.get("path").cloned() {
                    map.insert(
                        "Browser".into(),
                        serde_json::json!({ "target": { "File": path } }),
                    );
                } else {
                    map.insert("Html".into(), html);
                }
            }
            for nested in map.values_mut() {
                rewrite_html_tabs(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for nested in items {
                rewrite_html_tabs(nested);
            }
        }
        _ => {}
    }
}

/// Points the webview at the isolated named store for this data directory.
#[cfg(feature = "webview")]
pub(crate) fn apply_store<'a>(
    builder: wry::WebViewBuilder<'a>,
    data_dir: &Path,
) -> Result<wry::WebViewBuilder<'a>, String> {
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        if objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 14
        {
            let identifier = profile_identifier(data_dir).map_err(|e| e.to_string())?;
            Ok(builder.with_data_store_identifier(identifier))
        } else {
            // Named persistent stores are unavailable before macOS 14. Never fall
            // back to the shared default store; older systems use private browsing.
            Ok(builder.with_incognito(true))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data_dir;
        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserTarget, LocalScope, parse_url, profile_dir, rewrite_html_tabs, supported_file,
    };
    use std::path::Path;

    fn file_url(path: &str) -> String {
        url::Url::from_file_path(crate::test_path(path))
            .unwrap()
            .to_string()
    }

    fn file_scope(path: &str) -> LocalScope {
        LocalScope::of(&BrowserTarget::File(crate::test_path(path)))
    }

    #[test]
    fn navigation_checks_links_redirects_and_local_file_types() {
        let scope = file_scope("/tmp/page.html");
        let local = file_url("/tmp/page.html");
        for blocked in [
            "javascript:alert(1)",
            "data:text/html,hello",
            "ftp://example.com/a",
            "file:///etc/passwd",
            "https://user:password@example.com",
        ] {
            assert!(
                super::navigation_target(blocked, Some(&local), &scope).is_none(),
                "{blocked}"
            );
        }
        let url_scope = LocalScope::default();
        assert!(super::navigation_target(&local, None, &url_scope).is_none());
        assert!(super::navigation_target(&local, None, &scope).is_some());
        assert!(super::navigation_target("https://example.com/next", None, &url_scope).is_some());
    }

    #[test]
    fn address_bar_completes_bare_hosts_and_rejects_other_text() {
        let search = super::extras::DEFAULT_SEARCH_URL;
        let url = |input| match super::address_target(input, search) {
            Ok(BrowserTarget::Url(url)) => Some(url),
            _ => None,
        };
        assert_eq!(url("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(
            url(" docs.rs/serde?x=1 ").as_deref(),
            Some("https://docs.rs/serde?x=1")
        );
        assert_eq!(
            url("localhost:3000").as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(
            url("http://a.example/").as_deref(),
            Some("http://a.example/")
        );
        for rejected in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.c",
        ] {
            assert!(
                super::address_target(rejected, search).is_err(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn address_bar_searches_text_that_is_not_an_address() {
        let search = "https://search.example/?q=%s";
        for (input, expected) in [
            ("rust docs", "https://search.example/?q=rust+docs"),
            ("word", "https://search.example/?q=word"),
            (
                "user@example.com",
                "https://search.example/?q=user%40example.com",
            ),
        ] {
            assert_eq!(
                super::address_target(input, search),
                Ok(BrowserTarget::Url(expected.into())),
                "{input}"
            );
        }
        // An unusable template falls back to the default search.
        assert_eq!(
            super::address_target("word", "http://insecure/?q=%s"),
            Ok(BrowserTarget::Url("https://duckduckgo.com/?q=word".into()))
        );
    }

    #[test]
    fn tooltips_and_popups_hide_the_webview() {
        use eframe::egui::Order;
        assert!(super::covers_webview(Order::Tooltip));
        assert!(super::covers_webview(Order::Foreground));
        assert!(super::covers_webview(Order::Middle));
        assert!(!super::covers_webview(Order::Background));
    }

    #[test]
    fn remote_page_cannot_reach_other_local_files() {
        let scope = file_scope("/tmp/site/index.html");
        let remote = Some("https://example.com/");
        let sibling = file_url("/tmp/site/other.html");
        assert!(super::navigation_target(&sibling, remote, &scope).is_none());
        let downloads = file_url("/tmp/downloads/a.html");
        assert!(super::navigation_target(&downloads, remote, &scope).is_none());
        let opened = file_url("/tmp/site/index.html");
        assert!(
            super::navigation_target(&opened, remote, &scope).is_some(),
            "Back to the opened file stays allowed"
        );
    }

    #[test]
    fn local_page_reaches_only_files_under_the_opened_directory() {
        let scope = file_scope("/tmp/site/index.html");
        let local = file_url("/tmp/site/index.html");
        let nested = file_url("/tmp/site/docs/a.html");
        assert!(super::navigation_target(&nested, Some(&local), &scope).is_some());
        let outside = file_url("/tmp/other/a.html");
        assert!(super::navigation_target(&outside, Some(&local), &scope).is_none());
        let escape = format!("{}/../../other/a.html", file_url("/tmp/site/docs"));
        assert!(super::navigation_target(&escape, Some(&local), &scope).is_none());
        assert!(super::committed_target(&nested, &scope).is_some());
        assert!(super::committed_target(&outside, &scope).is_none());
    }

    #[test]
    fn profiles_are_stable_and_isolated_between_data_directories() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let id = super::profile_identifier(first.path()).unwrap();
        assert_eq!(id, super::profile_identifier(first.path()).unwrap());
        assert_ne!(id, super::profile_identifier(second.path()).unwrap());
    }

    #[test]
    fn html_tab_objects_become_browser_files() {
        let mut value = serde_json::json!({
            "primary": { "Html": { "path": "/page.html" } },
            "tabs": [{ "Html": { "path": "/other.html" } }]
        });
        rewrite_html_tabs(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "primary": { "Browser": { "target": { "File": "/page.html" } } },
                "tabs": [{ "Browser": { "target": { "File": "/other.html" } } }]
            })
        );
    }

    #[test]
    fn allowlist_accepts_html_files_and_http_urls() {
        assert!(supported_file(Path::new("docs/index.HTML")));
        assert!(supported_file(Path::new("page.xhtml")));
        assert!(!supported_file(Path::new("page.md")));
        assert!(parse_url("https://example.com/app").is_ok());
        assert!(parse_url("http://127.0.0.1:3000/").is_ok());
        assert!(parse_url("file:///etc/passwd").is_err());
        assert!(parse_url("javascript:alert(1)").is_err());
        assert!(parse_url("https://user:pass@example.com").is_err());
        assert_eq!(
            BrowserTarget::from_http_url("https://example.com")
                .unwrap()
                .title(),
            "https://example.com/"
        );
        assert_eq!(
            profile_dir(Path::new("/tmp/terminator-data")),
            Path::new("/tmp/terminator-data/webview")
        );
        assert!(
            super::href(&BrowserTarget::File(crate::test_path("/tmp/page.html")))
                .unwrap()
                .starts_with("file:")
        );
        assert!(super::href(&BrowserTarget::File("page.html".into())).is_err());
        let dir = tempfile::tempdir().unwrap();
        let profile = super::ensure_profile(dir.path()).unwrap();
        assert_eq!(profile, dir.path().join("webview"));
        assert!(profile.is_dir());
    }

    #[test]
    fn rewrite_leaves_unrelated_keys_and_malformed_html() {
        let mut value = serde_json::json!({
            "Image": { "path": "/pic.png" },
            "Html": { "other": true }
        });
        rewrite_html_tabs(&mut value);
        assert_eq!(value["Image"]["path"], "/pic.png");
        assert_eq!(value["Html"]["other"], serde_json::json!(true));
    }
}
