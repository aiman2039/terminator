//! Generate a deterministic one-, two-, or six-pane layout for isolated GUI smoke tests.
#![forbid(unsafe_code)]
use egui_dock::{DockState, NodeIndex};
fn main() {
    let tabs = std::env::args()
        .skip(1)
        .map(|id| serde_json::json!({"Terminal":id}))
        .collect::<Vec<_>>();
    assert!(matches!(tabs.len(), 1 | 2 | 6));
    let Some(first) = tabs.first() else {
        return;
    };
    let mut dock = DockState::new(vec![first.clone()]);
    if tabs.len() == 1 {
        print_dock(&dock);
        return;
    }
    let tree = dock.main_surface_mut();
    if tabs.len() == 2 {
        let Some(second) = tabs.get(1) else {
            return;
        };
        tree.split_right(NodeIndex::root(), 0.5, vec![second.clone()]);
        print_dock(&dock);
        return;
    }
    let Some(third) = tabs.get(2) else {
        return;
    };
    let [left, right] = tree.split_right(NodeIndex::root(), 0.333, vec![third.clone()]);
    let Some(second) = tabs.get(1) else {
        return;
    };
    tree.split_below(left, 0.5, vec![second.clone()]);
    let Some(fifth) = tabs.get(4) else {
        return;
    };
    let [center, right] = tree.split_right(right, 0.5, vec![fifth.clone()]);
    let Some(fourth) = tabs.get(3) else {
        return;
    };
    tree.split_below(center, 0.5, vec![fourth.clone()]);
    let Some(sixth) = tabs.get(5) else {
        return;
    };
    tree.split_below(right, 0.5, vec![sixth.clone()]);
    print_dock(&dock);
}

fn print_dock(dock: &DockState<serde_json::Value>) {
    let Ok(text) = serde_json::to_string(dock) else {
        return;
    };
    println!("{text}");
}
