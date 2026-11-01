//! Browser host for builds without the `webview` feature. Panes show an error
//! and the rest of the app is unchanged.
use crate::browser::{BrowserTarget, nav::NavState};
use eframe::egui;
use std::{collections::HashMap, collections::HashSet, path::Path};

const UNAVAILABLE: &str = "Embedded browser is not included in this build. Use Open in browser.";

pub(crate) struct VisibleBrowser {
    pub key: String,
    pub target: BrowserTarget,
    pub rect: egui::Rect,
}

pub(crate) struct SyncInput<'a> {
    pub ctx: &'a egui::Context,
    pub frame: &'a eframe::Frame,
    pub visible: &'a [VisibleBrowser],
    pub occluded: bool,
    pub covers: &'a [egui::Rect],
    pub data_dir: &'a Path,
}

pub(crate) struct BrowserHost {
    errors: HashMap<String, String>,
}

impl BrowserHost {
    pub fn new() -> Self {
        Self {
            errors: HashMap::new(),
        }
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        self.errors.get(key).map(String::as_str)
    }

    pub fn drop_view(&mut self, key: &str) {
        self.errors.remove(key);
    }

    pub fn retain(&mut self, keys: &HashSet<String>) {
        self.errors.retain(|key, _| keys.contains(key));
    }

    pub fn nav(&self, _key: &str) -> NavState {
        NavState::default()
    }

    pub fn stop(&self, _key: &str) {}

    pub fn navigate(&mut self, _key: &str, _target: &BrowserTarget) -> bool {
        false
    }

    pub fn committed(&mut self, _key: &str, _target: BrowserTarget) {}

    pub fn take_navigations(&self) -> Vec<(String, BrowserTarget)> {
        Vec::new()
    }

    pub fn go_back(&self, _key: &str) {}

    pub fn go_forward(&self, _key: &str) {}

    pub fn reload_view(&self, _key: &str) -> bool {
        false
    }

    pub fn take_opens(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    pub fn hide_all(&mut self) {}

    pub fn shutdown(&mut self) {
        self.errors.clear();
    }

    pub fn sync(&mut self, input: SyncInput<'_>) {
        for pane in input.visible {
            self.errors.insert(pane.key.clone(), UNAVAILABLE.into());
        }
    }
}
