use super::CaptureInput;
use super::{
    Harness, Options, PathBuf, Result, Value, capture, ensure, fs, id, json, prefs, session_ids,
    sessions,
};
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot).map_err(Into::into)
}
fn wait_sidebar(h: &Harness, ids: &Value) -> Result<()> {
    h.wait(
        |_| gui(h).is_ok_and(|s| s.get("sidebar_projects").unwrap_or(&Value::Null) == ids),
        8,
    )?;
    Ok(())
}
fn sort_sidebar(h: &Harness, o: &Options, first: &Value, second: &Value) -> Result<()> {
    let asc = json!([first["id"].clone(), second["id"].clone()]);
    let desc = json!([second["id"].clone(), first["id"].clone()]);
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "sorted-name-asc",
            actions: json!([]),
            after: 2200,
        },
        |_| wait_sidebar(h, &asc),
    )?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "sorted-name-desc",
            actions: json!([
                {"at_ms":800,"target":"project-sort"},
                {"at_ms":1600,"target":"sort-name-desc"}
            ]),
            after: 3200,
        },
        |_| wait_sidebar(h, &desc),
    )?;
    ensure!(
        prefs(h)?
            .get("project_sort")
            .is_some_and(|sort| sort == "name_desc"),
        "Name Z → A did not persist"
    );
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "sorted-name-desc-restart",
            actions: json!([]),
            after: 2200,
        },
        |_| wait_sidebar(h, &desc),
    )?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "sorted-latest-activity",
            actions: json!([
                {"at_ms":800,"target":"project-sort"},
                {"at_ms":1600,"target":"sort-latest-activity"},
                {"at_ms":2400,"target":format!("project-row:{}", id(second))}
            ]),
            after: 4200,
        },
        |_| wait_sidebar(h, &asc),
    )?;
    ensure!(
        prefs(h)?
            .get("project_sort")
            .is_some_and(|sort| sort == "latest_activity"),
        "Latest activity did not persist"
    );
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "sorted-latest-restart",
            actions: json!([]),
            after: 2200,
        },
        |_| wait_sidebar(h, &asc),
    )?;
    Ok(())
}
/// Two projects; the first holds a shell and a dirty editor tab.
struct Seeded {
    first: Value,
    second: Value,
    shell: Value,
    editor: Value,
    path: PathBuf,
    originals: [Value; 3],
}
pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = seed(&h)?;
    sort_sidebar(&h, o, &p.first, &p.second)?;
    remove_active_project(&h, o, &p)?;
    remove_all_projects(&h, o, &p.second)?;
    restore_project(&h, o, &p.first)?;
    check_restored(&h, &p)
}
fn seed(h: &Harness) -> Result<Seeded> {
    let first = h.project("first-project")?;
    let second = h.project("second-project")?;
    let shell = h.shell(&first)?;
    let other = h.shell(&second)?;
    let path = PathBuf::from(
        first
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    )
    .join("kept.md");
    fs::write(&path, "# Saved project file\n")?;
    let editor = h.editor(&first, &path)?;
    h.dirty_editor(&editor, "gg0C# Unsaved project file\u{1b}", 5)?;
    let shell_layout = h.layout(&first, std::slice::from_ref(&shell))?;
    let editor_layout = h.layout(&first, std::slice::from_ref(&editor))?;
    h.rpc(json!({"SaveLayout":{"project":id(&first),"layout":{
        "version":2,"active":"file","tabs":[
            {"id":"shell","primary":{"Terminal":id(&shell)},"layout":shell_layout},
            {"id":"file","primary":{"Terminal":id(&editor)},"layout":editor_layout}
        ]
    }}}))?;
    h.layout(&second, std::slice::from_ref(&other))?;
    h.rpc(json!({"SelectProject":{"project":id(&first)}}))?;
    let originals = [shell.clone(), other.clone(), editor.clone()];
    Ok(Seeded {
        first,
        second,
        shell,
        editor,
        path,
        originals,
    })
}
/// Wait until the GUI sidebar lists only `project`, selected.
fn wait_only_selected(h: &Harness, project: &Value) -> Result<()> {
    h.wait(
        |_| {
            gui(h).is_ok_and(|s| {
                s.get("sidebar_projects").unwrap_or(&Value::Null) == &json!([id(project)])
                    && s.get("selected_project").unwrap_or(&Value::Null)
                        == project.get("id").unwrap_or(&Value::Null)
            })
        },
        8,
    )?;
    Ok(())
}
/// Wait until the GUI sidebar is empty with nothing selected.
fn wait_empty(h: &Harness) -> Result<()> {
    h.wait(
        |_| {
            gui(h).is_ok_and(|s| {
                s.get("sidebar_projects").unwrap_or(&Value::Null) == &json!([])
                    && s.get("selected_project").is_none_or(Value::is_null)
            })
        },
        8,
    )?;
    Ok(())
}
fn remove_active_project(h: &Harness, o: &Options, p: &Seeded) -> Result<()> {
    let Seeded {
        first,
        second,
        editor,
        path,
        originals,
        ..
    } = p;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "removed-active-project",
            actions: json!([
                {"at_ms":1000,"target":format!("project-row:{}",id(first)),"right_click":true},
                {"at_ms":1700,"target":"Remove project from sidebar"}
            ]),
            after: 3200,
        },
        |_| wait_only_selected(h, second),
    )?;
    ensure!(
        prefs(h)?
            .get("hidden_projects")
            .ok_or_else(|| anyhow::anyhow!("missing hidden_projects"))?
            .as_array()
            .unwrap()
            .contains(
                first
                    .get("id")
                    .ok_or_else(|| anyhow::anyhow!("missing project id"))?,
            ),
        "Sidebar removal did not persist"
    );
    h.assert_pids(originals)?;
    ensure!(
        fs::read_to_string(path)? == "# Saved project file\n",
        "Removing a project changed its file"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(editor)}}))?
            .get("Text")
            .is_some_and(|text| text == "1"),
        "Removing a project discarded unsaved edits"
    );
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "removed-after-restart",
            actions: json!([]),
            after: 2200,
        },
        |_| wait_sidebar(h, &json!([id(second)])),
    )?;
    Ok(())
}
fn remove_all_projects(h: &Harness, o: &Options, second: &Value) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "all-projects-removed",
            actions: json!([
                {"at_ms":900,"target":format!("project-row:{}",id(second)),"right_click":true},
                {"at_ms":1600,"target":"Remove project from sidebar"}
            ]),
            after: 2800,
        },
        |_| wait_empty(h),
    )?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "empty-after-restart",
            actions: json!([]),
            after: 2200,
        },
        |_| wait_empty(h),
    )?;
    Ok(())
}
fn restore_project(h: &Harness, o: &Options, first: &Value) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "restored-project",
            actions: json!([
                {"at_ms":800,"target":"removed-projects"},
                {"at_ms":1600,"target":format!("restore-project:{}",id(first))}
            ]),
            after: 3200,
        },
        |_| wait_only_selected(h, first),
    )?;
    Ok(())
}
fn check_restored(h: &Harness, p: &Seeded) -> Result<()> {
    let Seeded {
        first,
        shell,
        editor,
        originals,
        ..
    } = p;
    let state = h.state()?;
    let projects = state
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing projects"))?;
    ensure!(
        projects.len() == 2 && sessions(&state).len() == 3,
        "Restoration duplicated or removed project/session records"
    );
    let restored = projects
        .iter()
        .find(|p| p.get("id").unwrap_or(&Value::Null) == first.get("id").unwrap_or(&Value::Null))
        .unwrap();
    ensure!(
        restored
            .get("layout")
            .and_then(|layout| layout.get("tabs"))
            .ok_or_else(|| anyhow::anyhow!("missing layout tabs"))?
            .as_array()
            .unwrap()
            .len()
            == 2,
        "Project tab layout was lost"
    );
    let ids = session_ids(&restored["layout"]);
    ensure!(
        ids.contains(
            &shell
                .get("id")
                .ok_or_else(|| anyhow::anyhow!("missing shell id"))?
                .as_str()
                .unwrap()
                .to_owned(),
        ) && ids.contains(
            &editor
                .get("id")
                .ok_or_else(|| anyhow::anyhow!("missing editor id"))?
                .as_str()
                .unwrap()
                .to_owned(),
        ),
        "Original panes were not restored"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(editor)}}))?
            .get("Text")
            .is_some_and(|text| text == "1"),
        "Restoration discarded unsaved edits"
    );
    h.assert_pids(originals)
}
