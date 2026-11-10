//! GUI-side Browser tab extras: address-bar search, zoom steps, find in
//! page, and the stack of closed tabs. Nothing here reads from the page:
//! find runs a script the GUI sends, and no page script can call the app.
use super::BrowserTarget;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

pub(crate) const DEFAULT_SEARCH_URL: &str = "https://duckduckgo.com/?q=%s";

/// Browser settings saved with the GUI preferences, not the daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserPrefs {
    /// https search URL; `%s` is the escaped address-bar text.
    pub search_url: String,
    /// Web inspector in Browser tabs (right click → Inspect Element).
    pub devtools: bool,
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        Self {
            search_url: DEFAULT_SEARCH_URL.into(),
            devtools: false,
        }
    }
}

/// The search URL for `query`, or `None` when `template` is not an https
/// URL with a `%s` placeholder.
pub(crate) fn search_url(template: &str, query: &str) -> Option<String> {
    let template = template.trim();
    if !template.contains("%s") {
        return None;
    }
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let url = super::parse_url(&template.replace("%s", &encoded)).ok()?;
    url.starts_with("https://").then_some(url)
}

pub(crate) fn valid_search_template(template: &str) -> bool {
    search_url(template, "test").is_some()
}

/// Search for `query` with the user's template, or the default when the
/// saved template is not usable.
pub(crate) fn search_target(template: &str, query: &str) -> Option<BrowserTarget> {
    search_url(template, query)
        .or_else(|| search_url(DEFAULT_SEARCH_URL, query))
        .map(BrowserTarget::Url)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZoomChange {
    In,
    Out,
    Reset,
}

const ZOOM_STEPS: &[f64] = &[
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The next zoom level, Chrome-style steps from 25% to 300%.
pub(crate) fn zoom_step(current: f64, change: ZoomChange) -> f64 {
    const EPSILON: f64 = 0.001;
    match change {
        ZoomChange::Reset => 1.0,
        ZoomChange::In => ZOOM_STEPS
            .iter()
            .copied()
            .find(|step| *step > current + EPSILON)
            .unwrap_or(current),
        ZoomChange::Out => ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < current - EPSILON)
            .unwrap_or(current),
    }
}

/// Label for a zoom level, such as `125%`.
pub(crate) fn zoom_label(level: f64) -> String {
    format!("{:.0}%", level * 100.0)
}

/// `window.find` call for the find bar. `fresh` starts a new search from
/// the top of the page. The query is a JSON string literal, never code.
pub(crate) fn find_script(query: &str, backwards: bool, fresh: bool) -> String {
    let literal = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
    let clear = if fresh {
        "window.getSelection().removeAllRanges();"
    } else {
        ""
    };
    format!(
        "(function(){{try{{{clear}window.find({literal},false,{backwards},true);}}catch(e){{}}}})();"
    )
}

/// Clears the find selection when the bar closes.
pub(crate) const FIND_CLEAR_SCRIPT: &str =
    "(function(){try{window.getSelection().removeAllRanges();}catch(e){}})();";

/// Find bar text per Browser tab key; a key is present while its bar is open.
#[derive(Clone, Debug, Default)]
pub(crate) struct FindBar {
    pub query: String,
    /// The query last sent to the page.
    pub sent: String,
}

const CLOSED_LIMIT: usize = 20;

/// Browser tabs seen last frame and those closed since, newest last.
#[derive(Debug, Default)]
pub(crate) struct ClosedTabs {
    known: HashMap<String, (String, BrowserTarget)>,
    closed: VecDeque<(String, BrowserTarget)>,
}

impl ClosedTabs {
    /// Record the open Browser tabs (key → project and target). A key seen
    /// before and gone now was closed.
    pub fn track(&mut self, open: HashMap<String, (String, BrowserTarget)>) {
        for (key, tab) in self.known.drain() {
            if !open.contains_key(&key) {
                self.closed.push_back(tab);
            }
        }
        while self.closed.len() > CLOSED_LIMIT {
            self.closed.pop_front();
        }
        self.known = open;
    }

    pub fn pop(&mut self) -> Option<(String, BrowserTarget)> {
        self.closed.pop_back()
    }

    pub fn is_empty(&self) -> bool {
        self.closed.is_empty()
    }
}

/// App state for the extras; zoom levels live in the browser host.
#[derive(Debug, Default)]
pub(crate) struct BrowserExtras {
    pub closed: ClosedTabs,
    pub find: HashMap<String, FindBar>,
    /// Find bar that takes keyboard focus next frame.
    pub focus_find: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_text_that_is_not_a_url_searches_with_https_template() {
        assert_eq!(
            search_url(DEFAULT_SEARCH_URL, "rust docs & more").as_deref(),
            Some("https://duckduckgo.com/?q=rust+docs+%26+more")
        );
        assert!(search_url("http://example.com/?q=%s", "x").is_none());
        assert!(search_url("https://example.com/", "x").is_none());
        assert!(search_url("javascript:%s", "x").is_none());
        assert!(valid_search_template("https://www.google.com/search?q=%s"));
        assert_eq!(
            search_target("not a url", "x"),
            Some(BrowserTarget::Url("https://duckduckgo.com/?q=x".into()))
        );
    }

    #[test]
    fn zoom_steps_up_and_down_and_stops_at_the_ends() {
        assert!((zoom_step(1.0, ZoomChange::In) - 1.1).abs() < 1e-9);
        assert!((zoom_step(1.0, ZoomChange::Out) - 0.9).abs() < 1e-9);
        assert!((zoom_step(3.0, ZoomChange::In) - 3.0).abs() < 1e-9);
        assert!((zoom_step(0.25, ZoomChange::Out) - 0.25).abs() < 1e-9);
        assert!((zoom_step(1.75, ZoomChange::Reset) - 1.0).abs() < 1e-9);
        assert_eq!(zoom_label(1.25), "125%");
    }

    #[test]
    fn find_script_quotes_the_query_as_a_string() {
        let script = find_script("a\"); alert(1); (\"", false, true);
        assert!(script.contains(r#"window.find("a\"); alert(1); (\"",false,false,true)"#));
        assert!(script.contains("removeAllRanges"));
        assert!(!find_script("x", true, false).contains("removeAllRanges"));
        assert!(find_script("x", true, false).contains("\"x\",false,true,true"));
    }

    #[test]
    fn closed_browser_tabs_reopen_newest_first() {
        let tab = |url: &str| ("p".to_owned(), BrowserTarget::Url(url.into()));
        let mut closed = ClosedTabs::default();
        closed.track(HashMap::from([
            ("a".into(), tab("https://a.example/")),
            ("b".into(), tab("https://b.example/")),
        ]));
        assert!(closed.is_empty());
        closed.track(HashMap::from([("b".into(), tab("https://b.example/"))]));
        closed.track(HashMap::new());
        assert_eq!(closed.pop(), Some(tab("https://b.example/")));
        assert_eq!(closed.pop(), Some(tab("https://a.example/")));
        assert_eq!(closed.pop(), None);
    }
}
