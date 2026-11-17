//! Native OS webview overlay. Lives on top of egui; must hide when covered.
use crate::browser::{
    self, BrowserTarget, LocalScope,
    nav::{self, NavState, NavStates},
    view::{Committed, Current, wry_error},
};
use anyhow::Result;
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
    time::Instant,
};

struct MountVisibleInput<'a> {
    frame: &'a eframe::Frame,
    visible: &'a [VisibleBrowser],
    covers: &'a [(egui::ViewportId, egui::Rect)],
    data_dir: &'a Path,
}

use wry::{
    Rect, WebContext, WebView,
    dpi::{LogicalPosition, LogicalSize},
};

#[path = "host_mount.rs"]
mod mount;

const MAX_VIEWS: usize = 4;
/// How often a shown view re-reads back/forward availability and its URL.
const HISTORY_POLL: std::time::Duration = std::time::Duration::from_millis(400);

pub(crate) struct VisibleBrowser {
    pub key: String,
    pub target: BrowserTarget,
    pub rect: egui::Rect,
    /// egui zoom (Interface size): native views take OS points.
    pub zoom: f32,
    /// The OS window that shows the pane: the main one or a floating one.
    pub viewport: egui::ViewportId,
}

struct Hosted {
    view: WebView,
    /// The window the native view is a child of.
    viewport: egui::ViewportId,
    target: BrowserTarget,
    used: Instant,
    history_at: Instant,
    url_at: Instant,
    scope: LocalScope,
    current: Current,
    committed: Committed,
    /// Last bounds and visibility sent to the native view.
    bounds: Option<egui::Rect>,
    shown: bool,
}

impl Hosted {
    /// Native calls only on change: the frame loop runs many times a second.
    fn show(&mut self, shown: bool) -> Result<(), wry::Error> {
        if self.shown != shown {
            self.view.set_visible(shown)?;
            self.shown = shown;
        }
        Ok(())
    }

    fn place(&mut self, rect: egui::Rect, zoom: f32) -> Result<(), wry::Error> {
        if self.bounds != Some(rect) {
            self.view.set_bounds(bounds(rect, zoom))?;
            self.bounds = Some(rect);
        }
        Ok(())
    }
}

pub(crate) struct BrowserHost {
    context: Option<WebContext>,
    views: HashMap<String, Hosted>,
    errors: HashMap<String, String>,
    opens: Arc<Mutex<Vec<(String, String)>>>,
    navigations: Arc<Mutex<Vec<(String, BrowserTarget)>>>,
    nav: NavStates,
    /// Page zoom per tab key, applied again when a view is rebuilt.
    zoom: HashMap<String, f64>,
    /// Web inspector for views built from now on.
    devtools: bool,
    scenes: browser::draw::Scenes,
    dark: bool,
}

pub(crate) struct SyncInput<'a> {
    pub ctx: &'a egui::Context,
    pub frame: &'a eframe::Frame,
    pub visible: &'a [VisibleBrowser],
    /// Rects of open egui popups and menus, each with its window. Native
    /// views draw above egui, so a pane under one of these hides until the
    /// popup closes. A dialog or center page covers all of its window.
    pub covers: &'a [(egui::ViewportId, egui::Rect)],
    pub data_dir: &'a Path,
}

impl BrowserHost {
    pub fn new() -> Self {
        Self {
            context: None,
            views: HashMap::new(),
            errors: HashMap::new(),
            opens: Arc::new(Mutex::new(Vec::new())),
            navigations: Arc::new(Mutex::new(Vec::new())),
            nav: NavStates::default(),
            zoom: HashMap::new(),
            devtools: false,
            scenes: browser::draw::Scenes::default(),
            dark: true,
        }
    }

    /// Page zoom of a tab; 1.0 until changed.
    pub fn zoom(&self, key: &str) -> f64 {
        self.zoom.get(key).copied().unwrap_or(1.0)
    }

    pub fn set_zoom(&mut self, key: &str, level: f64) {
        self.zoom.insert(key.to_owned(), level);
        if let Some(hosted) = self.views.get(key) {
            let _ = hosted.view.zoom(level);
        }
    }

    /// Runs a GUI-made script (find in page) in the tab's view. The page
    /// gets no way to call back into the app.
    pub fn run_script(&self, key: &str, script: &str) {
        if let Some(hosted) = self.views.get(key) {
            let _ = hosted.view.evaluate_script(script);
        }
    }

    /// A change rebuilds the views, so the inspector setting applies at once.
    pub fn set_devtools(&mut self, enabled: bool) {
        if self.devtools != enabled {
            self.devtools = enabled;
            self.views.clear();
        }
    }

    pub fn open_devtools(&self, key: &str) {
        if self.devtools
            && let Some(hosted) = self.views.get(key)
        {
            hosted.view.open_devtools();
        }
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        self.errors.get(key).map(String::as_str)
    }

    pub fn drop_view(&mut self, key: &str) {
        self.views.remove(key);
        self.errors.remove(key);
        self.nav.retain(|other| other != key);
    }

    pub fn retain(&mut self, keys: &HashSet<String>) {
        self.views.retain(|key, _| keys.contains(key));
        self.errors.retain(|key, _| keys.contains(key));
        self.zoom.retain(|key, _| keys.contains(key));
        self.scenes.retain(|key| keys.contains(key));
        self.nav.retain(|key| keys.contains(key));
    }

    /// Loading, title, and history flags for the toolbar and tab title.
    pub fn nav(&self, key: &str) -> NavState {
        self.nav.get(key)
    }

    pub fn stop(&self, key: &str) {
        if let Some(hosted) = self.views.get(key) {
            nav::stop(&hosted.view);
            self.nav.update(key, |state| state.loading = false);
        }
    }

    pub fn navigate(&mut self, key: &str, target: &BrowserTarget) -> bool {
        let Some(hosted) = self.views.get(key) else {
            return false;
        };
        let result = browser::href(target)
            .map_err(|e| e.to_string())
            .and_then(|href| hosted.view.load_url(&href).map_err(wry_error));
        if let Err(error) = result {
            self.errors.insert(key.into(), error);
        }
        true
    }

    pub fn committed(&mut self, key: &str, target: BrowserTarget) {
        if let Some(hosted) = self.views.get_mut(key) {
            hosted.target = target;
        }
    }

    pub fn take_navigations(&mut self) -> Vec<(String, BrowserTarget)> {
        let mut changes: HashMap<_, _> = self
            .navigations
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, _)| self.views.contains_key(key))
            .collect();
        // Also capture same-document navigation (e.g. history.pushState), which
        // does not necessarily produce a page-load callback.
        for (key, hosted) in &mut self.views {
            if hosted.url_at.elapsed() < HISTORY_POLL || !hosted.committed.load(Ordering::Acquire) {
                continue;
            }
            hosted.url_at = Instant::now();
            if let Ok(url) = hosted.view.url()
                && let Some(target) = browser::committed_target(&url, &hosted.scope)
                && target != hosted.target
            {
                if let Ok(mut current) = hosted.current.lock() {
                    *current = url;
                }
                changes.insert(key.clone(), target);
            }
        }
        changes.into_iter().collect()
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

    pub fn take_opens(&self) -> Vec<(String, String)> {
        self.opens
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default()
    }

    pub fn hide_all(&mut self) {
        for hosted in self.views.values_mut() {
            let _ = hosted.show(false);
        }
    }

    pub fn dismiss_download(&self, key: &str) {
        self.nav.update(key, |state| state.blocked_download = None);
    }

    pub fn shutdown(&mut self) {
        self.hide_all();
        self.views.clear();
        self.context = None;
        self.errors.clear();
        self.nav.retain(|_| false);
    }

    /// Drops the views of floating windows that `open` does not accept.
    /// Call it while a closed window still exists (the frame it closes).
    pub fn retain_windows(&mut self, open: impl Fn(egui::ViewportId) -> bool) {
        self.views
            .retain(|_, hosted| hosted.viewport == egui::ViewportId::ROOT || open(hosted.viewport));
    }

    pub fn sync(&mut self, input: SyncInput<'_>) {
        let SyncInput {
            ctx,
            frame,
            visible,
            covers,
            data_dir,
        } = input;
        self.nav.set_context(ctx);
        self.dark = ctx.global_style().visuals.dark_mode;
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
        self.mount_visible(MountVisibleInput {
            frame,
            visible,
            covers,
            data_dir,
        });
        self.hide_unlisted(visible);
        self.evict(visible);
        for (key, script) in self.scenes.take_scripts() {
            self.run_script(&key, &script);
        }
        self.pump_linux();
    }

    fn mount_visible(&mut self, input: MountVisibleInput<'_>) {
        let MountVisibleInput {
            frame,
            visible,
            covers,
            data_dir,
        } = input;
        for pane in visible {
            if pane.rect.width() < 32.0 || pane.rect.height() < 32.0 || covered(pane, covers) {
                if let Some(hosted) = self.views.get_mut(&pane.key) {
                    let _ = hosted.show(false);
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
        let page = (pane.key.as_str(), &pane.target);
        let href = browser::draw::pane_href(&self.scenes, page, (self.dark, data_dir))?;
        // A pane that moved to another window gets a new view there.
        if self
            .views
            .get(&pane.key)
            .is_some_and(|hosted| hosted.viewport != pane.viewport)
        {
            self.views.remove(&pane.key);
        }
        if let Some(hosted) = self.views.get_mut(&pane.key) {
            if hosted.target != pane.target {
                hosted.view.load_url(&href).map_err(wry_error)?;
                hosted.target = pane.target.clone();
            }
            hosted.place(pane.rect, pane.zoom).map_err(wry_error)?;
            hosted.show(true).map_err(wry_error)?;
            hosted.used = Instant::now();
            let loaded = self.nav.take_stale(&pane.key);
            if loaded {
                nav::refresh_favicon(&hosted.view, &self.nav, &pane.key);
            }
            if loaded || hosted.history_at.elapsed() >= HISTORY_POLL {
                hosted.history_at = Instant::now();
                nav::refresh_history(&hosted.view, &self.nav, &pane.key);
            }
            return Ok(());
        }
        self.mount(frame, pane, (href, data_dir))
    }

    fn hide_unlisted(&mut self, visible: &[VisibleBrowser]) {
        let listed: Vec<&str> = visible.iter().map(|pane| pane.key.as_str()).collect();
        for (key, hosted) in &mut self.views {
            if !listed.contains(&key.as_str()) {
                let _ = hosted.show(false);
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
}

/// Whether any popup rect of the pane's window overlaps the pane. Hidden
/// panes stay listed, so they keep their page and are not evicted.
fn covered(pane: &VisibleBrowser, covers: &[(egui::ViewportId, egui::Rect)]) -> bool {
    covers
        .iter()
        .any(|(viewport, cover)| *viewport == pane.viewport && cover.intersects(pane.rect))
}

fn eviction_victim<'a>(
    mounted: impl Iterator<Item = (&'a str, Instant)>,
    listed: &[&str],
) -> Option<String> {
    mounted
        .filter(|(key, _)| !listed.contains(key))
        .min_by_key(|(_, used)| *used)
        .map(|(key, _)| key.to_owned())
}

fn bounds(rect: egui::Rect, zoom: f32) -> Rect {
    let [x, y, w, h] = [rect.min.x, rect.min.y, rect.width(), rect.height()].map(|v| v * zoom);
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w, h).into(),
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
