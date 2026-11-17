use super::{VisibleBrowser, covered, egui, eviction_victim};
use crate::browser::BrowserTarget;
use std::time::{Duration, Instant};

fn pane(viewport: egui::ViewportId) -> VisibleBrowser {
    VisibleBrowser {
        key: "page".into(),
        target: BrowserTarget::Url("https://example.com/".into()),
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(400.0, 300.0)),
        zoom: 1.0,
        viewport,
    }
}

#[test]
fn popups_hide_only_the_panes_they_overlap() {
    let root = egui::ViewportId::ROOT;
    let pane = pane(root);
    let menu = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(200.0, 120.0));
    let elsewhere = egui::Rect::from_min_size(egui::pos2(600.0, 0.0), egui::vec2(100.0, 80.0));
    assert!(covered(&pane, &[(root, menu)]));
    assert!(!covered(&pane, &[(root, elsewhere)]));
    assert!(!covered(&pane, &[]));
}

#[test]
fn popup_in_another_window_does_not_hide_the_pane() {
    let float = egui::ViewportId::from_hash_of("float");
    let menu = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(200.0, 120.0));
    assert!(!covered(&pane(float), &[(egui::ViewportId::ROOT, menu)]));
    assert!(covered(&pane(float), &[(float, menu)]));
}

#[test]
fn eviction_only_removes_hidden_views() {
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
        None
    );
}
