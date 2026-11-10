//! Builds one native webview with the navigation, load, and download policy.
use crate::browser::{self, LocalScope, nav::NavStates};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

/// The view's last committed URL, shared with its navigation handler.
pub(crate) type Current = Arc<Mutex<String>>;

/// Set once the view commits its first navigation. Before that, WebKit's
/// `WKWebView.URL` is nil and wry's `WebView::url()` panics on it.
pub(crate) type Committed = Arc<AtomicBool>;

pub(crate) struct BuildInput<'a> {
    pub frame: &'a eframe::Frame,
    pub context: &'a mut WebContext,
    pub key: &'a str,
    pub href: &'a str,
    pub scope: &'a LocalScope,
    pub current: &'a Current,
    pub committed: &'a Committed,
    pub bounds: Rect,
    pub data_dir: &'a std::path::Path,
    pub nav: &'a NavStates,
    pub navigations: &'a Arc<Mutex<Vec<(String, browser::BrowserTarget)>>>,
    pub opens: &'a Arc<Mutex<Vec<(String, String)>>>,
    /// Web inspector (Settings → Browser developer tools; default off).
    pub devtools: bool,
}

pub(crate) fn build(input: BuildInput<'_>) -> Result<WebView, String> {
    let BuildInput {
        frame,
        context,
        key,
        href,
        scope,
        current,
        committed,
        bounds,
        data_dir,
        nav,
        navigations,
        opens,
        devtools,
    } = input;
    let builder = WebViewBuilder::new_with_web_context(context)
        .with_url(href)
        .with_devtools(devtools)
        .with_navigation_handler({
            let scope = scope.clone();
            let current = Arc::clone(current);
            move |url| {
                let current = current.lock().map(|c| c.clone()).ok();
                browser::navigation_target(&url, current.as_deref(), &scope).is_some()
            }
        })
        .with_on_page_load_handler(page_load_handler(PageLoad {
            key: key.to_owned(),
            scope: scope.clone(),
            current: Arc::clone(current),
            committed: Arc::clone(committed),
            states: nav.clone(),
            events: Arc::clone(navigations),
        }))
        .with_document_title_changed_handler({
            let states = nav.clone();
            let key = key.to_owned();
            move |title| {
                let title = title.trim().to_owned();
                states.update(&key, |state| {
                    state.title = (!title.is_empty()).then_some(title);
                });
            }
        })
        .with_bounds(bounds)
        .with_visible(true)
        .with_download_started_handler({
            let states = nav.clone();
            let key = key.to_owned();
            move |url, _| {
                states.update(&key, |state| state.blocked_download = Some(url));
                false
            }
        })
        .with_new_window_req_handler({
            let opens = Arc::clone(opens);
            let key = key.to_owned();
            move |url, _| {
                if browser::parse_url(&url).is_ok()
                    && let Ok(mut pending) = opens.lock()
                {
                    pending.push((key.clone(), url));
                }
                NewWindowResponse::Deny
            }
        });
    let builder = browser::apply_store(builder, data_dir)?;
    builder.build_as_child(frame).map_err(wry_error)
}

struct PageLoad {
    key: String,
    scope: LocalScope,
    current: Current,
    committed: Committed,
    states: NavStates,
    events: Arc<Mutex<Vec<(String, browser::BrowserTarget)>>>,
}

fn page_load_handler(load: PageLoad) -> impl Fn(PageLoadEvent, String) + 'static {
    let PageLoad {
        key,
        scope,
        current,
        committed,
        states,
        events,
    } = load;
    move |event, url| {
        committed.store(true, Ordering::Release);
        let loading = matches!(event, PageLoadEvent::Started);
        states.update(&key, |state| {
            state.loading = loading;
            state.stale = true;
            if loading {
                state.blocked_download = None;
            }
        });
        let Some(target) = browser::committed_target(&url, &scope) else {
            return;
        };
        if let Ok(mut current) = current.lock() {
            *current = url;
        }
        if matches!(event, PageLoadEvent::Finished)
            && let Ok(mut pending) = events.lock()
        {
            pending.push((key.clone(), target));
        }
    }
}

pub(crate) fn wry_error(error: wry::Error) -> String {
    format!("{error}. Use Open in browser.")
}
