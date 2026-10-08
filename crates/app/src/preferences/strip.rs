use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// IDE strip dock layouts per project. `DockState` has no `PartialEq`.
/// Equality, which gates preference saves, compares tabs, splits, fractions,
/// and focus. Runtime geometry (`rect`, `viewport`, `scroll`, window placement)
/// is ignored so a resize does not serialize every project dock on the GUI thread.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StripDocks(pub HashMap<String, egui_dock::DockState<crate::Tab>>);

impl PartialEq for StripDocks {
    fn eq(&self, other: &Self) -> bool {
        strip_docks_equal(&self.0, &other.0)
    }
}

fn strip_docks_equal(
    left: &HashMap<String, egui_dock::DockState<crate::Tab>>,
    right: &HashMap<String, egui_dock::DockState<crate::Tab>>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(project, dock)| {
            right
                .get(project)
                .is_some_and(|other| strip_dock_equal(dock, other))
        })
}

fn strip_dock_equal(
    left: &egui_dock::DockState<crate::Tab>,
    right: &egui_dock::DockState<crate::Tab>,
) -> bool {
    if left.focused_leaf() != right.focused_leaf() {
        return false;
    }
    let mut left_surfaces = left.iter_surfaces();
    let mut right_surfaces = right.iter_surfaces();
    loop {
        match (left_surfaces.next(), right_surfaces.next()) {
            (None, None) => return true,
            (Some(left), Some(right)) if strip_surface_equal(left, right) => {}
            _ => return false,
        }
    }
}

fn strip_surface_equal(
    left: &egui_dock::Surface<crate::Tab>,
    right: &egui_dock::Surface<crate::Tab>,
) -> bool {
    match (left.node_tree(), right.node_tree()) {
        (None, None) => true,
        (Some(left), Some(right)) => strip_tree_equal(left, right),
        _ => false,
    }
}

fn strip_tree_equal(
    left: &egui_dock::Tree<crate::Tab>,
    right: &egui_dock::Tree<crate::Tab>,
) -> bool {
    left.focused_leaf() == right.focused_leaf()
        && left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| strip_node_equal(left, right))
}

fn strip_node_equal(
    left: &egui_dock::Node<crate::Tab>,
    right: &egui_dock::Node<crate::Tab>,
) -> bool {
    match (left, right) {
        (egui_dock::Node::Empty, egui_dock::Node::Empty) => true,
        (egui_dock::Node::Leaf(left), egui_dock::Node::Leaf(right)) => {
            left.tabs == right.tabs
                && left.active == right.active
                && left.collapsed == right.collapsed
                && left.tab_bar_hidden == right.tab_bar_hidden
        }
        (egui_dock::Node::Vertical(left), egui_dock::Node::Vertical(right))
        | (egui_dock::Node::Horizontal(left), egui_dock::Node::Horizontal(right)) => {
            left.fraction == right.fraction
                && left.fully_collapsed == right.fully_collapsed
                && left.collapsed_leaf_count == right.collapsed_leaf_count
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::preferences::UiPreferences;
    use std::fs;

    #[test]
    fn strip_docks_round_trip_and_default_empty() {
        let prefs = UiPreferences::default();
        assert!(prefs.ide_strip_docks.0.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = UiPreferences::default();
        prefs.ide_strip_docks.0.insert(
            "a".into(),
            egui_dock::DockState::new(vec![crate::Tab::Terminal("x".into())]),
        );
        prefs.save(dir.path()).unwrap();
        let loaded = UiPreferences::load(dir.path()).unwrap();
        // Tab structure survives; viewport rects are runtime data that
        // sanitization resets (infinite fresh rects cannot round-trip JSON).
        let dock = loaded
            .ide_strip_docks
            .0
            .get("a")
            .expect("strip dock survives");
        assert!(dock.find_tab(&crate::Tab::Terminal("x".into())).is_some());
    }

    #[test]
    fn invalid_strip_docks_are_dropped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = UiPreferences::default();
        prefs.ide_strip_docks.0.insert(
            "a".into(),
            egui_dock::DockState::new(vec![crate::Tab::Terminal("x".into())]),
        );
        let mut value = serde_json::to_value(&prefs).unwrap();
        value["ide_strip_docks"]["a"]["surfaces"][0]["Main"]["focused_node"] =
            serde_json::json!(999);
        fs::write(
            dir.path().join("ui-preferences.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let loaded = UiPreferences::load(dir.path()).unwrap();
        assert!(loaded.ide_strip_docks.0.is_empty());
    }

    #[test]
    fn strip_dock_equality_ignores_runtime_geometry() {
        let dock = egui_dock::DockState::new(vec![crate::Tab::Terminal("x".into())]);
        let mut resized = dock.clone();
        let leaf = resized
            .main_surface_mut()
            .iter_mut()
            .next()
            .unwrap()
            .get_leaf_mut()
            .unwrap();
        leaf.set_rect(egui_dock::egui::Rect::from_min_size(
            egui_dock::egui::Pos2::ZERO,
            egui_dock::egui::vec2(40.0, 20.0),
        ));
        leaf.viewport = egui_dock::egui::Rect::from_min_size(
            egui_dock::egui::Pos2::new(1.0, 2.0),
            egui_dock::egui::vec2(30.0, 10.0),
        );
        leaf.scroll = 15.0;
        let mut left = UiPreferences::default();
        left.ide_strip_docks.0.insert("a".into(), dock.clone());
        let mut right = left.clone();
        right.ide_strip_docks.0.insert("a".into(), resized);
        assert_eq!(left, right);

        let mut focused = dock.clone();
        let path = focused.find_tab(&crate::Tab::Terminal("x".into())).unwrap();
        focused.set_focused_node_and_surface(path.node_path());
        right.ide_strip_docks.0.insert("a".into(), focused);
        assert_ne!(left, right);

        let mut split = dock.clone();
        split.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.25,
            vec![crate::Tab::Terminal("y".into())],
        );
        right.ide_strip_docks.0.insert("a".into(), split);
        assert_ne!(left, right);
    }
}
