//! GUI-owned browser tab identity. The native view is mounted later.
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

pub(crate) fn parse_url(value: &str) -> Result<String> {
    metadata::http_url(value)
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

/// Apply the same policy to initial loads, links, redirects, and history navigation.
pub(crate) fn navigation_target(value: &str, allow_local: bool) -> Option<BrowserTarget> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() == "file" && allow_local {
        let path = url.to_file_path().ok()?;
        return supported_file(&path).then_some(BrowserTarget::File(path));
    }
    BrowserTarget::from_http_url(value).ok()
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

#[cfg(test)]
mod tests {
    use super::{BrowserTarget, parse_url, profile_dir, rewrite_html_tabs, supported_file};
    use std::path::Path;

    #[test]
    fn navigation_checks_links_redirects_and_local_file_types() {
        for blocked in [
            "javascript:alert(1)",
            "data:text/html,hello",
            "ftp://example.com/a",
            "file:///etc/passwd",
            "https://user:password@example.com",
        ] {
            assert!(
                super::navigation_target(blocked, true).is_none(),
                "{blocked}"
            );
        }
        assert!(super::navigation_target("file:///tmp/page.html", false).is_none());
        assert!(super::navigation_target("file:///tmp/page.html", true).is_some());
        assert!(super::navigation_target("https://example.com/next", false).is_some());
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
            super::href(&BrowserTarget::File("/tmp/page.html".into()))
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
