//! Native OS webview overlay. Lives on top of egui; must hide when covered.
use crate::browser::{self, BrowserTarget};
use anyhow::Result;
use eframe::egui;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use wry::{
    NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
};

const MAX_VIEWS: usize = 4;
#[cfg(target_os = "macos")]
const DATA_STORE: [u8; 16] = *b"terminator.webv1";

pub(crate) struct VisibleBrowser {
    pub key: String,
    pub target: BrowserTarget,
    pub rect: egui::Rect,
}

struct Hosted {
    view: WebView,
    target: BrowserTarget,
    used: Instant,
}

pub(crate) struct BrowserHost {
    context: Option<WebContext>,
    views: HashMap<String, Hosted>,
    errors: HashMap<String, String>,
    opens: Arc<Mutex<Vec<String>>>,
}

pub(crate) struct SyncInput<'a> {
    pub frame: &'a eframe::Frame,
    pub visible: &'a [VisibleBrowser],
    pub occluded: bool,
    pub data_dir: &'a Path,
}

impl BrowserHost {
    pub fn new() -> Self {
        Self {
            context: None,
            views: HashMap::new(),
            errors: HashMap::new(),
            opens: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        self.errors.get(key).map(String::as_str)
    }

    pub fn drop_view(&mut self, key: &str) {
        self.views.remove(key);
        self.errors.remove(key);
    }

    pub fn go_back(&self, key: &str) {
        if let Some(hosted) = self.views.get(key) {
            let _ = hosted.view.go_back();
        }
    }

    pub fn go_forward(&self, key: &str) {
        if let Some(hosted) = self.views.get(key) {
            let _ = hosted.view.go_forward();
        }
    }

    pub fn reload_view(&self, key: &str) -> bool {
        self.views
            .get(key)
            .and_then(|hosted| hosted.view.reload().ok())
            .is_some()
    }

    pub fn take_opens(&self) -> Vec<String> {
        self.opens
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default()
    }

    pub fn hide_all(&mut self) {
        for hosted in self.views.values() {
            let _ = hosted.view.set_visible(false);
        }
    }

    pub fn shutdown(&mut self) {
        self.hide_all();
        self.views.clear();
        self.context = None;
        self.errors.clear();
    }

    pub fn sync(&mut self, input: SyncInput<'_>) {
        let SyncInput {
            frame,
            visible,
            occluded,
            data_dir,
        } = input;
        if occluded {
            self.hide_all();
            return;
        }
        if let Err(reason) = Self::platform_ok(frame) {
            self.hide_all();
            for pane in visible {
                self.errors.insert(pane.key.clone(), reason.clone());
            }
            return;
        }
        if let Err(error) = self.ensure_linux() {
            self.hide_all();
            for pane in visible {
                self.errors.insert(pane.key.clone(), error.clone());
            }
            return;
        }
        self.mount_visible(frame, visible, data_dir);
        self.hide_unlisted(visible);
        self.evict(visible);
        self.pump_linux();
    }

    fn mount_visible(
        &mut self,
        frame: &eframe::Frame,
        visible: &[VisibleBrowser],
        data_dir: &Path,
    ) {
        for pane in visible {
            if pane.rect.width() < 32.0 || pane.rect.height() < 32.0 {
                if let Some(hosted) = self.views.get(&pane.key) {
                    let _ = hosted.view.set_visible(false);
                }
                continue;
            }
            if let Err(error) = self.show_pane(frame, pane, data_dir) {
                self.views.remove(&pane.key);
                self.errors.insert(pane.key.clone(), error);
            } else {
                self.errors.remove(&pane.key);
            }
        }
    }

    fn show_pane(
        &mut self,
        frame: &eframe::Frame,
        pane: &VisibleBrowser,
        data_dir: &Path,
    ) -> Result<(), String> {
        let href = browser::href(&pane.target).map_err(|error| error.to_string())?;
        let bounds = bounds(pane.rect);
        if let Some(hosted) = self.views.get_mut(&pane.key) {
            if hosted.target != pane.target {
                hosted.view.load_url(&href).map_err(wry_error)?;
                hosted.target = pane.target.clone();
            }
            hosted.view.set_bounds(bounds).map_err(wry_error)?;
            hosted.view.set_visible(true).map_err(wry_error)?;
            hosted.used = Instant::now();
            return Ok(());
        }
        let view = self.create_view(frame, &href, bounds, data_dir)?;
        self.views.insert(
            pane.key.clone(),
            Hosted {
                view,
                target: pane.target.clone(),
                used: Instant::now(),
            },
        );
        Ok(())
    }

    fn create_view(
        &mut self,
        frame: &eframe::Frame,
        href: &str,
        bounds: Rect,
        data_dir: &Path,
    ) -> Result<WebView, String> {
        let profile = browser::ensure_profile(data_dir).map_err(|error| error.to_string())?;
        if self.context.is_none() {
            self.context = Some(WebContext::new(Some(profile)));
        }
        let context = self
            .context
            .as_mut()
            .ok_or_else(|| "Webview profile was not created".to_string())?;
        let builder = WebViewBuilder::new_with_web_context(context)
            .with_url(href)
            .with_bounds(bounds)
            .with_visible(true)
            .with_download_started_handler(|_, _| false)
            .with_new_window_req_handler({
                let opens = self.opens.clone();
                move |url, _| {
                    if browser::parse_url(&url).is_ok()
                        && let Ok(mut pending) = opens.lock()
                    {
                        pending.push(url);
                    }
                    NewWindowResponse::Deny
                }
            });
        let builder = apply_store(builder);
        builder.build_as_child(frame).map_err(wry_error)
    }

    fn hide_unlisted(&mut self, visible: &[VisibleBrowser]) {
        let listed: Vec<&str> = visible.iter().map(|pane| pane.key.as_str()).collect();
        for (key, hosted) in &self.views {
            if !listed.contains(&key.as_str()) {
                let _ = hosted.view.set_visible(false);
            }
        }
    }

    fn evict(&mut self, visible: &[VisibleBrowser]) {
        while self.views.len() > MAX_VIEWS {
            let listed: Vec<&str> = visible.iter().map(|pane| pane.key.as_str()).collect();
            let victim = eviction_victim(
                self.views
                    .iter()
                    .map(|(key, hosted)| (key.as_str(), hosted.used)),
                &listed,
            );
            let Some(key) = victim else {
                break;
            };
            self.views.remove(&key);
        }
    }

    fn platform_ok(frame: &eframe::Frame) -> Result<(), String> {
        let handle = frame
            .window_handle()
            .map_err(|error| format!("Embedded browser is not available: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::AppKit(_)
            | RawWindowHandle::Xlib(_)
            | RawWindowHandle::Xcb(_) => Ok(()),
            RawWindowHandle::Wayland(_) => Err(
                "Embedded browser needs X11 on Linux; Wayland is not supported yet. Use Open in browser."
                    .into(),
            ),
            _ => Err("Embedded browser is not available in this window. Use Open in browser.".into()),
        }
    }

    fn ensure_linux(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            if gtk::init().is_err() && !gtk::is_initialized() {
                return Err("Embedded browser could not start GTK. Use Open in browser.".into());
            }
        }
        let _ = self;
        Ok(())
    }

    fn pump_linux(&self) {
        #[cfg(target_os = "linux")]
        {
            while gtk::events_pending() {
                let _ = gtk::main_iteration_do(false);
            }
        }
        let _ = self;
    }
}

fn eviction_victim<'a>(
    mounted: impl Iterator<Item = (&'a str, Instant)>,
    listed: &[&str],
) -> Option<String> {
    let mounted: Vec<(&str, Instant)> = mounted.collect();
    mounted
        .iter()
        .copied()
        .filter(|(key, _)| !listed.contains(key))
        .min_by_key(|(_, used)| *used)
        .or_else(|| mounted.iter().copied().min_by_key(|(_, used)| *used))
        .map(|(key, _)| key.to_owned())
}

fn bounds(rect: egui::Rect) -> Rect {
    Rect {
        position: LogicalPosition::new(f64::from(rect.min.x), f64::from(rect.min.y)).into(),
        size: LogicalSize::new(f64::from(rect.width()), f64::from(rect.height())).into(),
    }
}

fn wry_error(error: wry::Error) -> String {
    format!("{error}. Use Open in browser.")
}

fn apply_store(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        builder.with_data_store_identifier(DATA_STORE)
    }
    #[cfg(not(target_os = "macos"))]
    builder
}

#[cfg(test)]
mod tests {
    use super::eviction_victim;
    use std::time::{Duration, Instant};

    #[test]
    fn eviction_prefers_unlisted_oldest_then_listed() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(1);
        let t2 = t1 + Duration::from_secs(1);
        let mounted = [("keep", t0), ("old", t1), ("newer", t2)];
        assert_eq!(
            eviction_victim(mounted.iter().copied(), &["keep", "newer"]).as_deref(),
            Some("old")
        );
        assert_eq!(
            eviction_victim(mounted.iter().copied(), &["keep", "old", "newer"]).as_deref(),
            Some("keep")
        );
    }
}
