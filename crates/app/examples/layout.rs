//! Generate a deterministic one-, two-, or six-pane layout for isolated GUI smoke tests.
use egui_dock::{DockState, NodeIndex};
fn main() {
    let tabs = std::env::args()
        .skip(1)
        .map(|id| serde_json::json!({"Terminal":id}))
        .collect::<Vec<_>>();
    assert!(matches!(tabs.len(), 1 | 2 | 6));
    let mut dock = DockState::new(vec![tabs[0].clone()]);
    if tabs.len() == 1 {
        println!("{}", serde_json::to_string(&dock).unwrap());
        return;
    }
    let tree = dock.main_surface_mut();
    if tabs.len() == 2 {
        tree.split_right(NodeIndex::root(), 0.5, vec![tabs[1].clone()]);
        println!("{}", serde_json::to_string(&dock).unwrap());
        return;
    }
    let [left, right] = tree.split_right(NodeIndex::root(), 0.333, vec![tabs[2].clone()]);
    tree.split_below(left, 0.5, vec![tabs[1].clone()]);
    let [center, right] = tree.split_right(right, 0.5, vec![tabs[4].clone()]);
    tree.split_below(center, 0.5, vec![tabs[3].clone()]);
    tree.split_below(right, 0.5, vec![tabs[5].clone()]);
    println!("{}", serde_json::to_string(&dock).unwrap());
}
