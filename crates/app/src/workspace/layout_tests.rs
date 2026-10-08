use super::model::Workspace;
use crate::Tab;
use egui_dock::DockState;
use std::collections::HashSet;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_browser_ids_migrate_once_and_primary_matches_the_pane() {
        let mut workspace = Workspace::empty();
        workspace.add("browser".into(), Tab::browser_file("/tmp/page.html".into()));
        let mut value = terminator_core::sanitize_layout(serde_json::to_value(workspace).unwrap());
        fn remove_browser_ids(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(browser) = map
                        .get_mut("Browser")
                        .and_then(serde_json::Value::as_object_mut)
                    {
                        browser.remove("id");
                    }
                    for nested in map.values_mut() {
                        remove_browser_ids(nested);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        remove_browser_ids(value);
                    }
                }
                _ => {}
            }
        }
        remove_browser_ids(&mut value);
        let loaded = Workspace::load(value).unwrap();
        let key = loaded.active_pane().unwrap().key();
        assert_eq!(loaded.tabs[0].primary.as_ref().unwrap().key(), key);
        let restored = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&loaded).unwrap(),
        ))
        .unwrap();
        assert_eq!(restored.active_pane().unwrap().key(), key);
    }

    #[test]
    fn emptying_every_pane_keeps_dock_access_usable() {
        // Regression: `:qa` closing the last pane left `tabs` empty, and
        // the next dock access panicked on `tabs[0]` in the Deref impls.
        let pane = Tab::Terminal("qa-last-pane".to_string());
        let mut workspace = Workspace::from_layout(DockState::new(vec![pane.clone()]));
        let path = workspace.tabs[0]
            .layout
            .find_tab(&pane)
            .expect("pane present");
        workspace.tabs[0].layout.remove_tab(path);
        workspace.drop_empty_tabs();
        assert_eq!(workspace.tabs.len(), 1);
        // Read access through Deref.
        assert!(workspace.iter_all_tabs().next().is_none());
        // Write access through DerefMut (the exact panic site).
        workspace.add("qa-next".to_string(), Tab::Terminal("qa-next".to_string()));
        assert!(workspace.contains(&Tab::Terminal("qa-next".to_string())));
    }

    #[test]
    fn missing_main_surface_is_rejected_before_pane_access() {
        let mut saved =
            terminator_core::sanitize_layout(serde_json::to_value(Workspace::empty()).unwrap());
        saved["tabs"][0]["layout"]["surfaces"] = serde_json::json!([]);
        let result = Workspace::load(saved);
        if let Ok(workspace) = &result {
            workspace.active_pane();
        }
        assert!(result.is_err());
    }

    #[test]
    fn invalid_focus_is_rejected_in_legacy_and_versioned_layouts() {
        let dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        let mut legacy = terminator_core::sanitize_layout(serde_json::to_value(&dock).unwrap());
        legacy["surfaces"][0]["Main"]["focused_node"] = serde_json::json!(999);
        assert!(
            Workspace::load(legacy.clone())
                .unwrap_err()
                .to_string()
                .contains("focus")
        );
        for version in [2, 3, 4, 5, 6, 7] {
            let mut saved = terminator_core::sanitize_layout(
                serde_json::to_value(Workspace::from_layout(dock.clone())).unwrap(),
            );
            saved["version"] = serde_json::json!(version);
            saved["tabs"][0]["layout"] = legacy.clone();
            assert!(
                Workspace::load(saved)
                    .unwrap_err()
                    .to_string()
                    .contains("focus")
            );
        }
    }

    #[test]
    fn media_layout_version_round_trips_and_preserves_legacy_tabs() {
        let mut workspace = Workspace::empty();
        workspace.add("shell".into(), Tab::Terminal("shell".into()));
        assert_eq!(workspace.version, 2);
        workspace.add(
            "image".into(),
            Tab::Image {
                path: "/image.png".into(),
            },
        );
        assert_eq!(workspace.version, 3);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Terminal("shell".into())));
        assert!(restored.contains(&Tab::Image {
            path: "/image.png".into()
        }));
        workspace.add("html".into(), Tab::browser_file("/page.html".into()));
        assert_eq!(workspace.version, 6);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::browser_file("/page.html".into())));
        workspace.add("player".into(), Tab::Player);
        assert_eq!(workspace.version, 6);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::Player));
        workspace.add(
            "native".into(),
            Tab::NativeEditor {
                path: "/notes/todo.md".into(),
            },
        );
        assert_eq!(workspace.version, 7);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::NativeEditor {
            path: "/notes/todo.md".into()
        }));
    }

    #[test]
    fn v4_html_tabs_migrate_to_browser() {
        let mut workspace = Workspace::empty();
        workspace.add("html".into(), Tab::browser_file("/page.html".into()));
        let mut saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        saved["version"] = serde_json::json!(4);
        demote_browser_tabs_to_html(&mut saved);
        let restored = Workspace::load(saved).unwrap();
        assert!(restored.contains(&Tab::browser_file("/page.html".into())));
        assert_eq!(restored.version, 6);
    }

    #[test]
    fn unsupported_layout_version_is_rejected() {
        let mut saved =
            terminator_core::sanitize_layout(serde_json::to_value(Workspace::empty()).unwrap());
        saved["version"] = serde_json::json!(8);
        assert!(
            Workspace::load(saved)
                .unwrap_err()
                .to_string()
                .contains("Unsupported")
        );
    }

    #[test]
    fn player_only_layout_stays_version_five() {
        let mut workspace = Workspace::empty();
        workspace.add("player".into(), Tab::Player);
        assert_eq!(workspace.version, 5);
        let saved = terminator_core::sanitize_layout(serde_json::to_value(&workspace).unwrap());
        let restored = Workspace::load(saved).unwrap();
        assert_eq!(restored.version, 5);
        assert!(restored.contains(&Tab::Player));
    }

    #[test]
    fn strip_player_removes_player_panes_and_keeps_other_tabs() {
        let mut workspace = Workspace::empty();
        workspace.add("shell".into(), Tab::Terminal("s".into()));
        workspace.add("player".into(), Tab::Player);
        workspace.strip_player();
        assert!(!workspace.contains(&Tab::Player));
        assert!(workspace.contains(&Tab::Terminal("s".into())));
    }

    #[test]
    fn strip_player_keeps_empty_workspace_identity() {
        let mut workspace = Workspace::empty();
        let before = serde_json::to_value(&workspace).unwrap();
        workspace.strip_player();
        workspace.remove_session("missing");
        assert_eq!(serde_json::to_value(&workspace).unwrap(), before);
    }

    #[test]
    fn emptied_workspace_keeps_its_identity() {
        let mut workspace = Workspace::empty();
        let id = workspace.active.clone();
        workspace.drop_empty_tabs();
        workspace.drop_empty_tabs();
        assert_eq!(workspace.active, id);
        assert_eq!(workspace.tabs[0].id, id);
    }

    fn demote_browser_tabs_to_html(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(browser) = map.remove("Browser") {
                    let path = browser
                        .get("target")
                        .and_then(|target| target.get("File"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    map.insert("Html".into(), serde_json::json!({ "path": path }));
                }
                for nested in map.values_mut() {
                    demote_browser_tabs_to_html(nested);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    demote_browser_tabs_to_html(nested);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn active_pane_is_none_for_an_emptied_dock() {
        let mut dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        let path = dock.find_tab(&Tab::Terminal("shell".into())).unwrap();
        dock.set_focused_node_and_surface(path.node_path());
        dock.remove_tab(path);
        let workspace = Workspace::from_layout(dock);
        assert!(workspace.active_pane().is_none());
    }

    #[test]
    fn legacy_splits_migrate_without_losing_sessions() {
        let mut dock = DockState::new(vec![Tab::Terminal("shell".into())]);
        dock.main_surface_mut().split_below(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("lower".into())],
        );
        let migrated = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&dock).unwrap(),
        ))
        .unwrap();
        assert_eq!(migrated.tabs.len(), 1);
        assert_eq!(migrated.iter_all_tabs().count(), 2);
        assert_eq!(
            migrated.main_surface().focused_leaf(),
            dock.main_surface().focused_leaf()
        );
    }
    #[test]
    fn top_level_tabs_keep_independent_splits_and_survive_restart() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("shell".into())]));
        let original = workspace.active.clone();
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.4,
            vec![Tab::Terminal("split".into())],
        );
        workspace.add("file-tab".into(), Tab::Terminal("editor".into()));
        assert_eq!(workspace.iter_all_tabs().count(), 1);
        assert!(workspace.activate_containing(&Tab::Terminal("split".into())));
        assert_eq!(workspace.active, original);
        assert_eq!(workspace.iter_all_tabs().count(), 2);
        let restored = Workspace::load(terminator_core::sanitize_layout(
            serde_json::to_value(&workspace).unwrap(),
        ))
        .unwrap();
        assert_eq!(restored.active, original);
        assert_eq!(restored.tabs.len(), 2);
        assert!(restored.contains(&Tab::Terminal("editor".into())));
    }
    #[test]
    fn add_at_inserts_between_existing_tabs_and_clamps() {
        let mut workspace = Workspace::empty();
        workspace.add("a".into(), Tab::Terminal("a".into()));
        workspace.add("c".into(), Tab::Terminal("c".into()));
        workspace.add_at(1, "b".into(), Tab::Terminal("b".into()));
        assert_eq!(
            workspace
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(workspace.active, "b");
        assert_eq!(workspace.ids(), vec!["a", "b", "c"]);
        assert_eq!(workspace.ids_before("b"), vec!["a"]);
        assert_eq!(workspace.ids_after("b"), vec!["c"]);
        workspace.add_at(99, "d".into(), Tab::Terminal("d".into()));
        assert_eq!(workspace.tabs.last().map(|tab| tab.id.as_str()), Some("d"));
        assert_eq!(workspace.ids_before("a"), Vec::<String>::new());
        assert_eq!(workspace.ids_after("d"), Vec::<String>::new());
    }

    #[test]
    fn closing_one_tab_keeps_other_layouts_and_focus() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("shell".into())]));
        let original = workspace.active.clone();
        workspace.add("file".into(), Tab::Terminal("editor".into()));
        workspace.remove_session("editor");
        assert_eq!(workspace.active, original);
        assert_eq!(workspace.iter_all_tabs().count(), 1);
        workspace.close(&original);
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.iter_all_tabs().count(), 0);
    }

    #[test]
    fn move_pane_to_group_leaf_swaps_single_panes_across_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("second".into(), Tab::Terminal("two".into()));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        // Workspace Deref addresses the active ("second") group; its leaves
        // are the drop targets.
        let leaves: Vec<egui_dock::NodePath> =
            workspace.iter_leaves().map(|(path, _)| path).collect();
        assert_eq!(leaves.len(), 2);
        let target = workspace
            .find_tab(&Tab::Terminal("three".into()))
            .unwrap()
            .node_path();
        // Both leaves hold a single pane, so the terminals exchange places
        // and both tabs survive with everything visible.
        assert!(workspace.move_pane_to_group_leaf(&Tab::Terminal("one".into()), "second", target));
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.tabs.len(), 2);
        let landed = workspace
            .find_tab(&Tab::Terminal("one".into()))
            .unwrap()
            .node_path();
        assert_eq!(landed, target);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
        let first_group = workspace
            .tabs
            .iter()
            .find(|tab| tab.id != workspace.active)
            .expect("source tab survives the swap");
        let swapped_home = first_group
            .layout
            .find_tab(&Tab::Terminal("three".into()))
            .expect("swapped pane stays in the source tab");
        assert_ne!(swapped_home.node_path(), target);
        assert!(!workspace.move_pane_to_group_leaf(
            &Tab::Terminal("ghost".into()),
            "second",
            target
        ));
        assert!(!workspace.move_pane_to_group_leaf(
            &Tab::Terminal("two".into()),
            "missing",
            target
        ));
    }

    #[test]
    fn move_pane_to_group_leaf_joins_stacked_leaves_and_drops_emptied_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        // Destination leaf already stacks two panes: the move joins it and
        // the emptied source tab is dropped.
        workspace.add("second".into(), Tab::Terminal("two".into()));
        workspace.tabs[1].layout = DockState::new(vec![
            Tab::Terminal("two".into()),
            Tab::Terminal("three".into()),
        ]);
        workspace.tabs[1].primary = Some(Tab::Terminal("two".into()));
        let target = workspace.tabs[1]
            .layout
            .find_tab(&Tab::Terminal("two".into()))
            .unwrap()
            .node_path();
        assert!(workspace.move_pane_to_group_leaf(&Tab::Terminal("one".into()), "second", target));
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, "second");
        let leaf = workspace.leaf(target).unwrap();
        assert_eq!(leaf.tabs.len(), 3);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
    }

    #[test]
    fn move_pane_to_split_opens_an_edge_split() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        let target = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        let group = workspace.active.clone();
        assert!(workspace.move_pane_to_split(
            &Tab::Terminal("left".into()),
            group.as_str(),
            target,
            egui_dock::Split::Right,
        ));
        // The emptied source leaf collapses: right splits into right+left
        // in separate leaves with the moved pane focused. (Leaf rectangles
        // only exist after rendering; geometric order is covered by the
        // headless UI drop test below.)
        assert_eq!(workspace.iter_leaves().count(), 2);
        let left_home = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        let right_home = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        assert_ne!(left_home, right_home);
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("left".into())));
    }

    #[test]
    fn move_pane_to_split_across_tabs_drops_the_emptied_tab() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("second".into(), Tab::Terminal("two".into()));
        let target = workspace
            .find_tab(&Tab::Terminal("two".into()))
            .unwrap()
            .node_path();
        assert!(workspace.move_pane_to_split(
            &Tab::Terminal("one".into()),
            "second",
            target,
            egui_dock::Split::Below,
        ));
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.iter_leaves().count(), 2);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(!workspace.move_pane_to_split(
            &Tab::Terminal("ghost".into()),
            "second",
            target,
            egui_dock::Split::Below,
        ));
    }

    #[test]
    fn move_pane_to_group_swaps_single_terminals_and_activates_destination() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        let first = workspace.active.clone();
        workspace.add("second".into(), Tab::Terminal("two".into()));
        // Both sides hold one terminal: they exchange places so both tabs
        // survive with everything visible.
        assert!(workspace.move_pane_to_group(&Tab::Terminal("one".into()), "second"));
        assert_eq!(workspace.active, "second");
        assert_eq!(workspace.tabs.len(), 2);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(workspace.contains(&Tab::Terminal("two".into())));
        assert_eq!(workspace.active_pane(), Some(&Tab::Terminal("one".into())));
        let origin = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == first)
            .expect("origin tab survives the swap");
        assert!(
            origin
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(!workspace.move_pane_to_group(&Tab::Terminal("ghost".into()), "second"));
        assert!(!workspace.move_pane_to_group(&Tab::Terminal("one".into()), "missing"));
    }

    #[test]
    fn move_pane_to_leaf_swaps_single_pane_splits() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        let leaves: Vec<egui_dock::NodePath> =
            workspace.iter_leaves().map(|(path, _)| path).collect();
        assert_eq!(leaves.len(), 2);
        let src = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        let dst = leaves.iter().copied().find(|path| *path != src).unwrap();
        assert!(workspace.move_pane_to_leaf(&Tab::Terminal("left".into()), dst));
        assert_eq!(workspace.iter_all_tabs().count(), 2);
        // Positions swapped: "left" now lives where "right" was.
        let now = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        assert_eq!(now, dst);
        let other = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        assert_eq!(other, src);
    }

    #[test]
    fn move_pane_to_new_group_at_end_creates_top_level_tab() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("two".into())],
        );
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("two".into()), workspace.tabs.len())
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(workspace.contains(&Tab::Terminal("one".into())));
        assert!(workspace.contains(&Tab::Terminal("two".into())));
        assert!(
            workspace
                .move_pane_to_new_group_at(&Tab::Terminal("ghost".into()), 0)
                .is_none()
        );
    }

    #[test]
    fn move_pane_to_new_group_at_lands_between_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // Dropping "two" at the front removes its group; the new tab
        // lands first with "one" behind it.
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("two".into()), 0)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert!(
            workspace
                .move_pane_to_new_group_at(&Tab::Terminal("ghost".into()), 0)
                .is_none()
        );
    }

    #[test]
    fn move_pane_to_new_group_at_keeps_trailing_slot_when_source_empties() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // Dropping "one" past the end removes its group; the trailing slot
        // shifts down so the new tab still lands last.
        let id = workspace
            .move_pane_to_new_group_at(&Tab::Terminal("one".into()), 2)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 2);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert_eq!(workspace.active, id);
    }

    #[test]
    fn move_pane_to_new_group_at_keeps_source_slot_when_source_survives() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        // "three" leaves its group behind, so no slot shifts: the new tab
        // lands at the end and the source group keeps its panes.
        workspace
            .move_pane_to_new_group_at(&Tab::Terminal("three".into()), 3)
            .expect("new group");
        assert_eq!(workspace.tabs.len(), 3);
        assert!(
            workspace.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert!(
            workspace.tabs[2]
                .layout
                .find_tab(&Tab::Terminal("three".into()))
                .is_some()
        );
    }

    #[test]
    fn reorder_group_moves_top_level_tabs() {
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        let first = workspace.tabs[0].id.clone();
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        workspace.add("tC".into(), Tab::Terminal("three".into()));
        assert!(workspace.reorder_group("tC", 0));
        assert_eq!(
            workspace.ids(),
            vec!["tC".to_owned(), first.clone(), "tB".to_owned()]
        );
        // Focus follows the tab, not the slot.
        assert_eq!(workspace.active, "tC");
        assert!(workspace.reorder_group(&first, 2));
        assert_eq!(
            workspace.ids(),
            vec!["tC".to_owned(), "tB".to_owned(), first.clone()]
        );
        assert!(!workspace.reorder_group("ghost", 0));
    }

    #[test]
    fn adopt_dock_reuses_a_blank_tab_and_appends_beside_content() {
        let mut workspace = Workspace::empty();
        let layout = DockState::new(vec![Tab::Terminal("shell".into())]);
        assert!(workspace.adopt_dock(layout, true));
        assert_eq!(workspace.tabs.len(), 1);
        assert!(workspace.contains(&Tab::Terminal("shell".into())));

        workspace.add("editor".into(), Tab::Terminal("edit".into()));
        let editor = workspace.active.clone();
        assert!(workspace.adopt_dock(DockState::new(vec![Tab::Terminal("other".into())]), false));
        assert_eq!(workspace.tabs.len(), 3);
        assert_eq!(workspace.active, editor);
        assert!(workspace.contains(&Tab::Terminal("other".into())));

        let mut ids = HashSet::new();
        ids.insert("shell".into());
        ids.insert("missing".into());
        workspace.remove_terminal_ids(&ids);
        assert!(!workspace.contains(&Tab::Terminal("shell".into())));
        assert!(workspace.contains(&Tab::Terminal("edit".into())));
        assert!(workspace.contains(&Tab::Terminal("other".into())));
    }

    #[test]
    fn detach_file_window_moves_pane_to_fresh_top_level_tab() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        // Detach tears the file window off into a new tab at the end of
        // the strip; the source tab keeps the remaining terminal.
        let id = workspace
            .move_pane_to_new_group_at(&editor, workspace.tabs.len())
            .expect("detach creates a tab");
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active, id);
        assert!(workspace.tabs[0].layout.find_tab(&shell).is_some());
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_none());
        assert!(workspace.tabs[1].layout.find_tab(&editor).is_some());
        assert!(
            !workspace
                .move_pane_to_new_group_at(
                    &Tab::NativeEditor {
                        path: "/tmp/ghost.md".into()
                    },
                    0
                )
                .is_some()
        );
    }

    #[test]
    fn take_pane_for_float_drops_emptied_tab_and_reports_home() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        // A lone pane's tab goes away; the home id survives for dock-back.
        workspace.add("lone".into(), Tab::Terminal("solo".into()));
        let home = workspace
            .take_pane(&Tab::Terminal("solo".into()))
            .expect("pane present");
        assert_eq!(home, "lone");
        assert!(workspace.tabs.iter().all(|tab| tab.id != "lone"));
        // Taking from a shared leaf keeps the tab with the other pane.
        let home = workspace.take_pane(&editor).expect("pane present");
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&shell).is_some());
        assert!(workspace.take_pane(&editor).is_none());
    }

    #[test]
    fn take_pane_of_last_pane_keeps_one_empty_group_for_dock_access() {
        let shell = Tab::Terminal("one".into());
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        let home = workspace.take_pane(&shell).expect("pane present");
        assert_eq!(home, workspace.active);
        // Not a new tab: the emptied group is replaced by the single empty
        // group the dock accessors require, holding zero panes. The pane
        // itself survives in the float window for dock-back.
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.iter_all_tabs().count(), 0);
    }

    #[test]
    fn dock_back_float_prefers_home_then_active_leaf() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        let home = workspace.tabs[0].id.clone();
        // Home group still exists: the pane stacks there, no new tab.
        workspace.dock_back(editor.clone(), &home);
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        // Home group gone: the pane lands in the active leaf instead.
        let mut workspace = Workspace::from_layout(DockState::new(vec![shell.clone()]));
        workspace.dock_back(editor.clone(), "missing");
        assert_eq!(workspace.tabs.len(), 1);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        // Already docked: focuses instead of duplicating.
        let before = workspace.tabs[0].layout.iter_all_tabs().count();
        workspace.dock_back(editor.clone(), "missing");
        assert_eq!(workspace.tabs[0].layout.iter_all_tabs().count(), before);
    }

    #[test]
    fn dock_back_file_window_returns_to_source_group() {
        let shell = Tab::Terminal("one".into());
        let editor = Tab::NativeEditor {
            path: "/tmp/note.md".into(),
        };
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![shell.clone(), editor.clone()]));
        workspace
            .move_pane_to_new_group_at(&editor, workspace.tabs.len())
            .expect("detach creates a tab");
        let home = workspace.tabs[0].id.clone();
        // Both tabs hold a single pane, so dock-back exchanges them: the
        // file window returns to its tab and selects it.
        assert!(workspace.move_pane_to_group(&editor, &home));
        assert_eq!(workspace.active, home);
        assert!(workspace.tabs[0].layout.find_tab(&editor).is_some());
        assert!(workspace.tabs[1].layout.find_tab(&shell).is_some());
        assert!(!workspace.move_pane_to_group(&editor, "missing"));
    }
}
