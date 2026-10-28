use super::*;

#[test]
fn legacy_project_playlists_merge_into_default() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":1,"player_playlists":{"a":["/a/one.mp3"],"b":["/b/two.mp3","/a/one.mp3"]}}"#,
    )
    .unwrap();
    let prefs = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(prefs.selected_playlist, "Default");
    assert_eq!(
        prefs.selected_tracks(),
        [PathBuf::from("/a/one.mp3"), PathBuf::from("/b/two.mp3")].as_slice()
    );
    assert!(prefs.player_playlists.is_empty());
    prefs.save(dir.path()).unwrap();
    let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
    assert!(!raw.contains("player_playlists"));
    assert!(raw.contains("Default"));
}

#[test]
fn per_side_tools_round_trip_and_default_to_native_views() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(fresh.left_tool, None);
    assert!(!fresh.right_projects);
    // Files written before per-side tools existed load the same way.
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":2,"ide_mode":true}"#,
    )
    .unwrap();
    let legacy = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(legacy.left_tool, None);
    assert!(!legacy.right_projects);
    let mut prefs = legacy;
    prefs.left_tool = Some(SidebarTool::Git);
    prefs.right_projects = true;
    prefs.save(dir.path()).unwrap();
    let loaded = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(loaded.left_tool, Some(SidebarTool::Git));
    assert!(loaded.right_projects);
    // Layout presets carry the per-side assignment too.
    let mut saved = loaded;
    saved.save_current_layout("both-sides");
    assert_eq!(
        saved.named_layouts["both-sides"].left_tool,
        Some(SidebarTool::Git)
    );
    assert!(saved.named_layouts["both-sides"].right_projects);
    saved.left_tool = None;
    saved.right_projects = false;
    saved.apply_layout("both-sides").unwrap();
    assert_eq!(saved.left_tool, Some(SidebarTool::Git));
    assert!(saved.right_projects);
}

#[test]
fn problems_and_structure_tools_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut prefs = UiPreferences::load(dir.path()).unwrap();
    prefs.left_tool = Some(SidebarTool::Problems);
    prefs.tool = SidebarTool::Structure;
    prefs.save(dir.path()).unwrap();
    let loaded = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(loaded.left_tool, Some(SidebarTool::Problems));
    assert_eq!(loaded.tool, SidebarTool::Structure);
}

#[test]
fn hosts_tool_round_trips_and_unknown_tools_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut prefs = UiPreferences::load(dir.path()).unwrap();
    prefs.tool = SidebarTool::Hosts;
    prefs.left_tool = Some(SidebarTool::Hosts);
    prefs.hosts_filter = "prod".into();
    prefs.save(dir.path()).unwrap();
    let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
    assert!(raw.contains(r#""tool":"hosts""#) || raw.contains(r#""tool": "hosts""#));
    let loaded = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(loaded.tool, SidebarTool::Hosts);
    assert_eq!(loaded.left_tool, Some(SidebarTool::Hosts));
    assert_eq!(loaded.hosts_filter, "prod");
    // A tool from a newer build keeps the rest of the preferences.
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":2,"tool":"future","left_tool":"future","history_filter":"kept","named_layouts":{"x":{"ide_mode":true,"ide_terminal_collapsed":false,"visible":true,"left_visible":true,"tool":"future","left_agents":false,"left_tool":"Vnc","sidebar_width":300.0,"left_width":200.0}}}"#,
    )
    .unwrap();
    let future = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(future.tool, SidebarTool::Explorer);
    assert_eq!(future.left_tool, None);
    assert_eq!(future.history_filter, "kept");
    assert_eq!(future.named_layouts["x"].tool, SidebarTool::Explorer);
    assert_eq!(future.named_layouts["x"].left_tool, Some(SidebarTool::Vnc));
    // Older files without the Hosts fields load unchanged.
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":2,"tool":"Vnc"}"#,
    )
    .unwrap();
    let old = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(old.tool, SidebarTool::Vnc);
    assert!(old.hosts_filter.is_empty());
}

#[test]
fn ide_mode_is_persisted_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let prefs = UiPreferences::load(dir.path()).unwrap();
    assert!(prefs.ide_mode, "fresh preferences default to IDE layout");
    assert!(!prefs.ide_terminal_collapsed);
    let mut prefs = prefs;
    prefs.ide_mode = true;
    prefs.ide_terminal_collapsed = true;
    prefs.save(dir.path()).unwrap();
    let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
    assert!(raw.contains("ide_mode"));
    assert!(UiPreferences::load(dir.path()).unwrap().ide_mode);
    assert!(
        UiPreferences::load(dir.path())
            .unwrap()
            .ide_terminal_collapsed
    );
    // Stale version-1 files written before version-2 still open with IDE mode off,
    // and the load migrates them so the reset does not re-fire.
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":1,"ide_mode":true}"#,
    )
    .unwrap();
    let migrated = UiPreferences::load(dir.path()).unwrap();
    assert!(!migrated.ide_mode);
    assert_eq!(migrated.version, 2);
    migrated.save(dir.path()).unwrap();
    let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
    assert!(raw.contains(r#""version": 2"#));
    let mut reopened = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(reopened.version, 2);
    reopened.ide_mode = true;
    reopened.save(dir.path()).unwrap();
    assert!(UiPreferences::load(dir.path()).unwrap().ide_mode);
}

#[test]
fn ide_sidebar_height_defaults_full_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let prefs = UiPreferences::load(dir.path()).unwrap();
    assert!(prefs.ide_sidebars_full_height);
    let mut prefs = prefs;
    prefs.ide_mode = true;
    prefs.ide_sidebars_full_height = false;
    prefs.save_current_layout("wide");
    prefs.ide_sidebars_full_height = true;
    prefs.apply_layout("wide").unwrap();
    assert!(!prefs.ide_sidebars_full_height);
    prefs.save(dir.path()).unwrap();
    assert!(
        !UiPreferences::load(dir.path())
            .unwrap()
            .ide_sidebars_full_height
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap())
            .unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("ide_sidebars_full_height");
    value["named_layouts"]["wide"]
        .as_object_mut()
        .unwrap()
        .remove("ide_sidebars_full_height");
    fs::write(
        dir.path().join("ui-preferences.json"),
        serde_json::to_string(&value).unwrap(),
    )
    .unwrap();
    let loaded = UiPreferences::load(dir.path()).unwrap();
    assert!(loaded.ide_sidebars_full_height);
    assert!(
        loaded
            .named_layouts
            .get("wide")
            .unwrap()
            .ide_sidebars_full_height
    );
}

#[test]
fn playlists_can_be_created_renamed_and_deleted() {
    let mut prefs = UiPreferences::default();
    prefs.ensure_default_playlist();
    assert!(prefs.create_playlist("Focus"));
    assert!(!prefs.create_playlist("Focus"));
    assert_eq!(prefs.selected_playlist, "Focus");
    assert!(prefs.rename_selected_playlist("Deep"));
    assert_eq!(prefs.selected_playlist, "Deep");
    prefs.delete_selected_playlist();
    assert_eq!(prefs.selected_playlist, "Default");
    prefs.delete_selected_playlist();
    assert!(prefs.playlists.is_empty());
    prefs.ensure_default_playlist();
    assert_eq!(prefs.selected_playlist, "Default");
}

#[test]
fn old_preferences_request_attention_migration_once() {
    let old: UiPreferences =
        serde_json::from_str(r#"{"version":1,"typography_migrated":true}"#).unwrap();
    assert!(!old.attention_migrated);
    assert!(old.left_visible);
    assert!(!old.left_agents);
    assert!(old.typography_migrated);
    assert!(old.markdown_modes.is_empty());
    assert!(old.hidden_projects.is_empty());
    assert_eq!(old.project_sort, ProjectSort::NameAsc);
    assert!(old.project_activity.is_empty());
    assert_eq!(old.history_sort, HistorySort::LatestActivity);
    assert!(old.history_filter.is_empty());
    assert!(old.info_process_open);
    assert!(old.info_resources_open);
    assert!(old.info_show_system);
}

#[test]
fn restart_preserves_independent_expansion_sidebar_and_migration() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = UiPreferences::default();
    assert!(p.history_expanded.is_empty());
    p.history_expanded.insert("a".into(), true);
    p.expanded.insert("a".into(), false);
    p.expanded.insert("b".into(), true);
    p.toggle(SidebarTool::Agents);
    p.toggle(SidebarTool::Agents);
    p.setup_completed = true;
    p.left_agents = true;
    p.width = 410.0;
    p.typography_migrated = true;
    p.attention_migrated = true;
    p.hidden_projects.insert("hidden-project".into());
    p.project_sort = ProjectSort::LatestActivity;
    p.project_filter = "term".into();
    p.project_activity.insert("a".into(), 42);
    p.history_sort = HistorySort::NameDesc;
    p.history_filter = "term".into();
    p.markdown_modes
        .insert("editor-a".into(), crate::markdown::Mode::Split);
    p.markdown_modes
        .insert("editor-b".into(), crate::markdown::Mode::Preview);
    p.save(dir.path()).unwrap();
    assert_eq!(p, UiPreferences::load(dir.path()).unwrap());
    p.toggle(SidebarTool::Git);
    assert!(p.visible);
}

#[test]
fn unknown_version_is_not_overwritten_and_width_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ui-preferences.json"), r#"{"version":3}"#).unwrap();
    assert!(UiPreferences::load(dir.path()).is_err());
    fs::write(dir.path().join("ui-preferences.json"), r#"{"width":900}"#).unwrap();
    assert_eq!(UiPreferences::load(dir.path()).unwrap().width, 480.0);
}

#[test]
fn legacy_pascal_case_tool_loads() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("ui-preferences.json"),
        r#"{"version":2,"tool":"Git","visible":true}"#,
    )
    .unwrap();
    let prefs = UiPreferences::load(dir.path()).unwrap();
    assert_eq!(prefs.tool, SidebarTool::Git);
    assert!(prefs.visible);
}

#[test]
fn saved_tool_uses_snake_case() {
    let dir = tempfile::tempdir().unwrap();
    let prefs = UiPreferences {
        tool: SidebarTool::Git,
        ..UiPreferences::default()
    };
    prefs.save(dir.path()).unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("ui-preferences.json")).unwrap()).unwrap();
    assert_eq!(raw.get("tool"), Some(&serde_json::json!("git")));
}

#[test]
fn setup_waits_for_inventory_and_counts_hidden_projects() {
    let mut preferences = UiPreferences::default();
    assert!(!preferences.needs_setup(false, 0));
    assert!(preferences.needs_setup(true, 0));
    preferences.hidden_projects.insert("hidden".into());
    assert!(!preferences.needs_setup(true, 1));
    preferences.setup_completed = true;
    assert!(!preferences.needs_setup(true, 0));
}
