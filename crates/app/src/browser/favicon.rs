//! Browser tab icons. The GUI downloads them itself, without the page's
//! cookies, and hands the bytes to egui's image loaders.
use crate::{App, Job};
use eframe::egui;
use std::sync::OnceLock;
use std::time::Duration;

/// Icons above this size are ignored; real favicons are a few KiB.
const MAX_BYTES: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

/// Download state for one icon URL.
pub(crate) enum Favicon {
    Loading,
    /// egui image URI and the bytes its loaders decode.
    Ready(String, egui::load::Bytes),
    Failed,
}

pub(crate) async fn download(url: &str) -> Option<Vec<u8>> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .user_agent(format!("terminator/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()
        })
        .as_ref()?;
    let response = client.get(url).send().await.ok()?;
    let html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("html"));
    if !response.status().is_success()
        || html
        || response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    (bytes.len() <= MAX_BYTES && decodable(&bytes)).then(|| bytes.to_vec())
}

/// Only keep bytes egui can paint, so a tab never shows a load-error glyph.
fn decodable(bytes: &[u8]) -> bool {
    is_svg(bytes) || image::load_from_memory(bytes).is_ok()
}

fn is_svg(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes.get(..bytes.len().min(256)).unwrap_or_default()).contains("<svg")
}

/// SVG needs its extension for the SVG loader; the image loader guesses
/// every other format from the bytes.
fn image_uri(index: usize, bytes: &[u8]) -> String {
    let svg = if is_svg(bytes) { ".svg" } else { "" };
    format!("bytes://favicon/{index}{svg}")
}

impl App {
    /// Starts the icon download for a shown browser tab, once per URL.
    pub(crate) fn request_favicon(&mut self, key: &str) {
        if let Some(url) = self.browser_host.nav(key).favicon
            && !self.favicons.contains_key(&url)
            && self.jobs.send(Job::Favicon(url.clone())).is_ok()
        {
            self.favicons.insert(url, Favicon::Loading);
        }
    }

    /// The tab's downloaded icon, ready to paint.
    pub(crate) fn browser_favicon(&self, key: &str) -> Option<egui::ImageSource<'static>> {
        let url = self.browser_host.nav(key).favicon?;
        match self.favicons.get(&url) {
            Some(Favicon::Ready(uri, bytes)) => Some(egui::ImageSource::Bytes {
                uri: uri.clone().into(),
                bytes: bytes.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn apply_favicon(&mut self, url: String, bytes: Option<Vec<u8>>) {
        let state = match bytes {
            Some(bytes) => {
                let uri = image_uri(self.favicons.len(), &bytes);
                Favicon::Ready(uri, bytes.into())
            }
            None => Favicon::Failed,
        };
        self.favicons.insert(url, state);
    }
}

#[cfg(test)]
mod tests {
    use super::image_uri;

    #[test]
    fn svg_icons_keep_an_svg_uri() {
        assert_eq!(
            image_uri(3, br#"<?xml version="1.0"?><svg xmlns="x"/>"#),
            "bytes://favicon/3.svg"
        );
        assert_eq!(image_uri(4, b"\0\0\x01\0"), "bytes://favicon/4");
    }
}
