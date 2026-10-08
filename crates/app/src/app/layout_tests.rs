#[cfg(test)]
use egui_dock::DockState;
use egui_dock::NodeIndex;
use terminator_core::*;

use super::super::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrendered_layout_roundtrips_all_tabs() {
        let mut dock = DockState::new(vec![Tab::Terminal("first".into())]);
        dock.main_surface_mut().split_right(
            NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("second".into())],
        );
        let json = sanitize_layout(serde_json::to_value(&dock).unwrap());
        let restored: DockState<Tab> = serde_json::from_value(json).unwrap();
        assert_eq!(restored.iter_all_tabs().count(), 2);
        assert!(restored.find_tab(&Tab::Terminal("second".into())).is_some());
    }
}
