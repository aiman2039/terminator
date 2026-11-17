//! Builds a pane's native view in its OS window, and the platform checks.
use super::{BrowserHost, Hosted, VisibleBrowser, bounds};
use crate::browser::{
    self, LocalScope,
    view::{self, Committed, Current},
};
use eframe::egui;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use wry::{
    WebContext,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
};

impl BrowserHost {
    /// Makes the view as a child of the pane's window: the main window, or
    /// the floating window that shows the pane.
    pub(super) fn mount(
        &mut self,
        frame: &eframe::Frame,
        pane: &VisibleBrowser,
        (href, data_dir): (String, &Path),
    ) -> Result<(), String> {
        let float;
        let window: &dyn HasWindowHandle = if pane.viewport == egui::ViewportId::ROOT {
            frame
        } else if let Some(found) = egui_winit::viewport_window(pane.viewport) {
            float = found;
            &float
        } else {
            // The floating window is not ready; the next frame mounts it.
            return Ok(());
        };
        let scope = LocalScope::of(&pane.target);
        let current: Current = Arc::new(Mutex::new(href.clone()));
        let committed = Committed::default();
        let profile = browser::ensure_profile(data_dir).map_err(|error| error.to_string())?;
        let context = self
            .context
            .get_or_insert_with(|| WebContext::new(Some(profile)));
        let view = view::build(view::BuildInput {
            window,
            context,
            key: &pane.key,
            href: &href,
            scope: &scope,
            current: &current,
            committed: &committed,
            bounds: bounds(pane.rect, pane.zoom),
            data_dir,
            nav: &self.nav,
            navigations: &self.navigations,
            opens: &self.opens,
            devtools: self.devtools,
            scenes: &self.scenes,
        })?;
        if let Some(level) = self.zoom.get(&pane.key) {
            let _ = view.zoom(*level);
        }
        self.views.insert(
            pane.key.clone(),
            Hosted {
                view,
                viewport: pane.viewport,
                target: pane.target.clone(),
                used: Instant::now(),
                history_at: Instant::now(),
                url_at: Instant::now(),
                scope,
                current,
                committed,
                bounds: Some(pane.rect),
                shown: true,
            },
        );
        Ok(())
    }

    pub(super) fn platform_ok(frame: &eframe::Frame) -> Result<(), String> {
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

    pub(super) fn ensure_linux(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            if gtk::init().is_err() && !gtk::is_initialized() {
                return Err("Embedded browser could not start GTK. Use Open in browser.".into());
            }
        }
        let _ = self;
        Ok(())
    }

    pub(super) fn pump_linux(&self) {
        #[cfg(target_os = "linux")]
        {
            while gtk::events_pending() {
                let _ = gtk::main_iteration_do(false);
            }
        }
        let _ = self;
    }
}
