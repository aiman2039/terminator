use super::{covered, egui, eviction_victim};
use std::time::{Duration, Instant};

#[test]
fn popups_hide_only_the_panes_they_overlap() {
    let pane = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(400.0, 300.0));
    let menu = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(200.0, 120.0));
    let elsewhere = egui::Rect::from_min_size(egui::pos2(600.0, 0.0), egui::vec2(100.0, 80.0));
    assert!(covered(pane, &[menu]));
    assert!(!covered(pane, &[elsewhere]));
    assert!(!covered(pane, &[]));
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
