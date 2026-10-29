//! Per-view page state the browser toolbar reads: loading, title, history.
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wry::WebView;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NavState {
    pub loading: bool,
    pub title: Option<String>,
    pub back: bool,
    pub forward: bool,
    /// A load started or finished since the last history query.
    pub stale: bool,
    /// http(s) icon URL the loaded page declares, or its `/favicon.ico`.
    pub favicon: Option<String>,
}

/// Shared with webview callbacks, which run outside the egui frame. A change
/// requests a repaint so the toolbar shows it without waiting for input.
#[derive(Clone, Default)]
pub(crate) struct NavStates {
    states: Arc<Mutex<HashMap<String, NavState>>>,
    repaint: Arc<OnceLock<egui::Context>>,
}

impl NavStates {
    pub fn set_context(&self, ctx: &egui::Context) {
        let _ = self.repaint.set(ctx.clone());
    }

    pub fn get(&self, key: &str) -> NavState {
        self.states
            .lock()
            .ok()
            .and_then(|states| states.get(key).cloned())
            .unwrap_or_default()
    }

    pub fn update(&self, key: &str, change: impl FnOnce(&mut NavState)) {
        let Ok(mut states) = self.states.lock() else {
            return;
        };
        let state = states.entry(key.to_owned()).or_default();
        let before = state.clone();
        change(state);
        if *state != before
            && let Some(ctx) = self.repaint.get()
        {
            ctx.request_repaint();
        }
    }

    /// Whether the view's history flags need a fresh query; clears the mark.
    pub fn take_stale(&self, key: &str) -> bool {
        self.states
            .lock()
            .ok()
            .and_then(|mut states| {
                states
                    .get_mut(key)
                    .map(|state| std::mem::take(&mut state.stale))
            })
            .unwrap_or(false)
    }

    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Ok(mut states) = self.states.lock() {
            states.retain(|key, _| keep(key));
        }
    }
}

/// Bit 1 = can go back, bit 2 = can go forward. Engines without the
/// Navigation API report both, so the buttons stay usable.
const HISTORY_JS: &str = "typeof navigation === 'undefined' ? 3 : \
    (navigation.canGoBack ? 1 : 0) + (navigation.canGoForward ? 2 : 0)";

/// Refreshes back/forward availability. wry has no safe native query, so ask
/// the page; the answer arrives later through the shared state.
pub(crate) fn refresh_history(view: &WebView, states: &NavStates, key: &str) {
    let states = states.clone();
    let key = key.to_owned();
    let _ = view.evaluate_script_with_callback(HISTORY_JS, move |result| {
        let bits = result.trim().parse::<u8>().unwrap_or(3);
        states.update(&key, |state| {
            state.back = bits & 1 != 0;
            state.forward = bits & 2 != 0;
        });
    });
}

/// The page's declared icon, else `/favicon.ico` on http(s) origins.
const FAVICON_JS: &str = "(() => { \
    const link = document.querySelector('link[rel~=\"icon\"]'); \
    if (link && link.href) return link.href; \
    return /^https?:$/.test(location.protocol) ? location.origin + '/favicon.ico' : ''; \
})()";

/// Reads the page icon URL after a load. Only plain http(s) URLs are kept;
/// the GUI downloads the bytes itself, outside the page.
pub(crate) fn refresh_favicon(view: &WebView, states: &NavStates, key: &str) {
    let states = states.clone();
    let key = key.to_owned();
    let _ = view.evaluate_script_with_callback(FAVICON_JS, move |result| {
        let favicon = favicon_url(&result);
        states.update(&key, |state| state.favicon = favicon);
    });
}

/// Decodes the script's JSON string result into an allowed icon URL.
fn favicon_url(result: &str) -> Option<String> {
    let url = serde_json::from_str::<String>(result).ok()?;
    crate::browser::parse_url(&url).ok()
}

pub(crate) fn stop(view: &WebView) {
    let _ = view.evaluate_script("window.stop()");
}

/// Splits an http(s) URL into scheme prefix, host, and the rest, so the
/// address bar can emphasize the host the way Chrome does.
pub(crate) fn split_host(url: &str) -> Option<(&str, &str, &str)> {
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|scheme| url.starts_with(scheme))?;
    let rest = url.get(scheme.len()..)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((scheme, rest.get(..end)?, rest.get(end..)?))
}

#[cfg(test)]
mod tests {
    use super::{favicon_url, split_host};

    #[test]
    fn favicon_accepts_only_web_urls() {
        assert_eq!(
            favicon_url(r#""https://example.com/favicon.ico""#).as_deref(),
            Some("https://example.com/favicon.ico")
        );
        assert_eq!(favicon_url(r#""""#), None);
        assert_eq!(favicon_url(r#""data:image/png;base64,AAAA""#), None);
        assert_eq!(favicon_url(r#""file:///etc/passwd""#), None);
        assert_eq!(favicon_url("not json"), None);
    }

    #[test]
    fn address_bar_emphasizes_only_the_host() {
        assert_eq!(
            split_host("https://login.example.com/common?x=1"),
            Some(("https://", "login.example.com", "/common?x=1"))
        );
        assert_eq!(
            split_host("http://example.com"),
            Some(("http://", "example.com", ""))
        );
        assert_eq!(split_host("file:///tmp/a.html"), None);
        assert_eq!(split_host("not a url"), None);
    }
}
