use super::*;
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}
fn wait_sidebar(h: &Harness, ids: &Value) -> Result<()> {
    h.wait(|_| gui(h).is_ok_and(|s| s["sidebar_projects"] == *ids), 8)?;
    Ok(())
}
fn sort_sidebar(h: &Harness, o: &Options, first: &Value, second: &Value) -> Result<()> {
    let asc = json!([first["id"].clone(), second["id"].clone()]);
    let desc = json!([second["id"].clone(), first["id"].clone()]);
    capture(h, o, "sorted-name-asc", json!([]), 2200, |_| {
        wait_sidebar(h, &asc)
    })?;
    capture(
        h,
        o,
        "sorted-name-desc",
        json!([
            {"at_ms":800,"target":"project-sort"},
            {"at_ms":1600,"target":"sort-name-desc"}
        ]),
        3200,
        |_| wait_sidebar(h, &desc),
    )?;
    ensure!(
        prefs(h)?["project_sort"] == "name_desc",
        "Name Z → A did not persist"
    );
    capture(h, o, "sorted-name-desc-restart", json!([]), 2200, |_| {
        wait_sidebar(h, &desc)
    })?;
    capture(
        h,
        o,
        "sorted-latest-activity",
        json!([
            {"at_ms":800,"target":"project-sort"},
            {"at_ms":1600,"target":"sort-latest-activity"},
            {"at_ms":2400,"target":format!("project-row:{}", id(second))}
        ]),
        4200,
        |_| wait_sidebar(h, &asc),
    )?;
    ensure!(
        prefs(h)?["project_sort"] == "latest_activity",
        "Latest activity did not persist"
    );
    capture(h, o, "sorted-latest-restart", json!([]), 2200, |_| {
        wait_sidebar(h, &asc)
    })?;
    Ok(())
}
pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let first = h.project("first-project")?;
    let second = h.project("second-project")?;
    let shell = h.shell(&first)?;
    let other = h.shell(&second)?;
    let path = PathBuf::from(first["path"].as_str().unwrap()).join("kept.md");
    fs::write(&path, "# Saved project file\n")?;
    let editor = h.editor(&first, &path)?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok()
        },
        5,
    )?;
    h.write(&mut h.attach(&editor)?, "gg0C# Unsaved project file\u{1b}")?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok_and(|s| s["Text"] == "1")
        },
        5,
    )?;
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
    sort_sidebar(&h, o, &first, &second)?;

    capture(
        &h,
        o,
        "removed-active-project",
        json!([
            {"at_ms":1000,"target":format!("project-row:{}",id(&first)),"right_click":true},
            {"at_ms":1700,"target":"Remove project from sidebar"}
        ]),
        3200,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["sidebar_projects"] == json!([id(&second)])
                            && s["selected_project"] == second["id"]
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        prefs(&h)?["hidden_projects"]
            .as_array()
            .unwrap()
            .contains(&first["id"]),
        "Sidebar removal did not persist"
    );
    h.assert_pids(&originals)?;
    ensure!(
        fs::read_to_string(&path)? == "# Saved project file\n",
        "Removing a project changed its file"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))?["Text"] == "1",
        "Removing a project discarded unsaved edits"
    );

    capture(&h, o, "removed-after-restart", json!([]), 2200, |_| {
        h.wait(
            |_| gui(&h).is_ok_and(|s| s["sidebar_projects"] == json!([id(&second)])),
            8,
        )?;
        Ok(())
    })?;
    capture(
        &h,
        o,
        "all-projects-removed",
        json!([
            {"at_ms":900,"target":format!("project-row:{}",id(&second)),"right_click":true},
            {"at_ms":1600,"target":"Remove project from sidebar"}
        ]),
        2800,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["sidebar_projects"] == json!([]) && s["selected_project"].is_null()
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    capture(&h, o, "empty-after-restart", json!([]), 2200, |_| {
        h.wait(
            |_| {
                gui(&h).is_ok_and(|s| {
                    s["sidebar_projects"] == json!([]) && s["selected_project"].is_null()
                })
            },
            8,
        )?;
        Ok(())
    })?;

    capture(
        &h,
        o,
        "restored-project",
        json!([
            {"at_ms":800,"target":"removed-projects"},
            {"at_ms":1600,"target":format!("restore-project:{}",id(&first))}
        ]),
        3200,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["sidebar_projects"] == json!([id(&first)])
                            && s["selected_project"] == first["id"]
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    let state = h.state()?;
    ensure!(
        state["projects"].as_array().unwrap().len() == 2 && sessions(&state).len() == 3,
        "Restoration duplicated or removed project/session records"
    );
    let restored = state["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == first["id"])
        .unwrap();
    ensure!(
        restored["layout"]["tabs"].as_array().unwrap().len() == 2,
        "Project tab layout was lost"
    );
    let ids = session_ids(&restored["layout"]);
    ensure!(
        ids.contains(&shell["id"].as_str().unwrap().to_owned())
            && ids.contains(&editor["id"].as_str().unwrap().to_owned()),
        "Original panes were not restored"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))?["Text"] == "1",
        "Restoration discarded unsaved edits"
    );
    h.assert_pids(&originals)
}
