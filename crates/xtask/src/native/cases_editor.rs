use super::super::harness::{Harness, git, id, session, session_ids, sessions};
use super::dispatch::{Options, capture, plain, prefs, setup};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::fs;
pub(crate) fn inline_rename(o: &Options) -> Result<()> {
    let (h, _, s, _) = setup("inline-titles")?;
    let aux = s.get(1).ok_or_else(|| anyhow!("missing session"))?;
    plain(
        &h,
        o,
        "renamed",
        json!([{"at_ms":1000,"target":"workspace-tab:Terminal 1","right_click":true},{"at_ms":1300,"target":"Rename terminal…"},{"at_ms":1700,"target":"rename-input","text":"Build workspace"},{"at_ms":2000,"target":"rename-input","key":"Enter"},{"at_ms":2500,"target":"terminal","right_click":true},{"at_ms":2800,"target":"Rename terminal…"},{"at_ms":3200,"target":"rename-input","text":"Worker pane"},{"at_ms":3500,"target":"rename-input","key":"Enter"},{"at_ms":4000,"target":format!("session-row:{}",id(aux)),"right_click":true},{"at_ms":4300,"target":"Rename terminal…"},{"at_ms":4700,"target":"rename-input","text":"Aux shell"},{"at_ms":5000,"target":"rename-input","key":"Enter"}]),
        6400,
    )?;
    let state = h.state()?;
    for (index, label) in [(0, "Build workspace"), (5, "Worker pane"), (1, "Aux shell")] {
        let renamed = s.get(index).ok_or_else(|| anyhow!("missing session"))?;
        ensure!(
            session(&state, id(renamed))["label"] == label,
            "Rename targeted wrong session"
        );
    }
    plain(
        &h,
        o,
        "editing-inline",
        json!([{"at_ms":900,"target":"workspace-tab:Build workspace","right_click":true},{"at_ms":1200,"target":"Rename terminal…"},{"at_ms":1500,"target":"rename-input","text":"Editing inline"}]),
        2300,
    )?;
    h.assert_pids(&s)
}
pub(crate) fn editor_lifecycle(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("editor-isolation")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    let shell = s.get(5).ok_or_else(|| anyhow!("missing session"))?;
    let actions = json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":5000,"target":format!("session-row:{}",id(shell)),"right_click":true},{"at_ms":5400,"target":"Rename terminal…"},{"at_ms":5900,"target":"rename-input","text":"Workspace shell"},{"at_ms":6400,"target":"rename-input","key":"Enter"},{"at_ms":6900,"target":format!("session-row:{}",id(shell))}]);
    capture(&h, o, "editor-lifecycle", actions, 8500, |_| {
        let state = h.wait(|s| sessions(s).iter().any(|s| s["kind"] == "editor"), 5)?;
        let editor = sessions(&state)
            .iter()
            .find(|s| s["kind"] == "editor")
            .unwrap();
        ensure!(
            !s.iter().any(|s| s["pid"] == editor["pid"]),
            "Editor reused shell PTY"
        );
        h.wait(
            |s| {
                s.get("projects")
                    .and_then(|projects| projects.get(0))
                    .and_then(|project| project.get("layout"))
                    .is_some_and(|layout| session_ids(layout).len() == 7)
            },
            5,
        )?;
        h.write(&mut h.attach(editor)?, ":q\r")?;
        h.wait(|s| session(s, id(editor))["lifecycle"] == "ended", 5)?;
        h.wait(
            |s| {
                s.get("projects")
                    .and_then(|projects| projects.get(0))
                    .and_then(|project| project.get("layout"))
                    .is_some_and(|layout| session_ids(layout).len() == 6)
            },
            5,
        )?;
        Ok(())
    })?;
    ensure!(
        session(
            &h.state()?,
            id(s.get(5).ok_or_else(|| anyhow!("missing session"))?),
        )["label"]
            == "Workspace shell",
        "Sidebar rename failed"
    );
    h.assert_pids(&s)
}
pub(crate) fn file_close(o: &Options) -> Result<()> {
    let (h, _, shells, root) = setup("file-close")?;
    let source = root.join("source.rs");
    fs::write(&source, "fn main() {}\n")?;
    let close_log = plain(
        &h,
        o,
        "clean-file-close",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":1250,"target":"explorer-file:source.rs"},{"at_ms":2700,"target":"workspace-close:source.rs"}]),
        4000,
    )?;
    let state = h.state()?;
    let editors = sessions(&state)
        .iter()
        .filter(|s| s["kind"] == "editor")
        .collect::<Vec<_>>();
    ensure!(
        editors.len() == 1
            && editors
                .first()
                .is_some_and(|editor| editor["lifecycle"] == "ended"),
        "Double click or clean close failed: editor lifecycles {:?}; {close_log}",
        editors
            .iter()
            .map(|editor| &editor["lifecycle"])
            .collect::<Vec<_>>()
    );
    capture(
        &h,
        o,
        "dirty-file-close",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2700,"target":"workspace-close:source.rs"},{"at_ms":3400,"target":"Cancel"},{"at_ms":4100,"target":"workspace-close:source.rs"},{"at_ms":4700,"target":"Save and close"}]),
        6500,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st)
                        .iter()
                        .any(|s| s["kind"] == "editor" && s["lifecycle"] == "running")
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| s["kind"] == "editor" && s["lifecycle"] == "running")
                .unwrap();
            h.write(&mut h.attach(editor)?, "iUnsaved change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r.get("Text")
                                .and_then(Value::as_str)
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(source)?.contains("Unsaved change"),
        "Save-and-close lost buffer"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state)
            .iter()
            .filter(|s| s["kind"] == "editor")
            .all(|s| s["lifecycle"] == "ended"),
        "Editor did not close"
    );
    ensure!(
        session_ids(
            state
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
        )
        .len()
            == 6,
        "Close removed shells"
    );
    let keep = root.join("keep.rs");
    fs::write(&keep, "fn keep() {}\n")?;
    capture(
        &h,
        o,
        "dirty-file-discard",
        json!([
            {"at_ms":1100,"target":"explorer-file:keep.rs"},
            {"at_ms":2700,"target":"workspace-close:keep.rs"},
            {"at_ms":3600,"target":"Discard changes"}
        ]),
        5500,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st).iter().any(|s| {
                        s["kind"] == "editor"
                            && s["lifecycle"] == "running"
                            && s["label"] == "keep.rs"
                    })
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| {
                    s["kind"] == "editor" && s["lifecycle"] == "running" && s["label"] == "keep.rs"
                })
                .unwrap();
            h.write(&mut h.attach(editor)?, "iDiscarded change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r.get("Text")
                                .and_then(Value::as_str)
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(&keep)? == "fn keep() {}\n",
        "Discard wrote the buffer"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state)
            .iter()
            .filter(|s| s["kind"] == "editor")
            .all(|s| s["lifecycle"] == "ended"),
        "Discard did not close the editor"
    );
    ensure!(
        session_ids(
            state
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
        )
        .len()
            == 6,
        "Discard close removed shells"
    );
    let bar = root.join("bar.rs");
    fs::write(&bar, "fn bar() {}\n")?;
    capture(
        &h,
        o,
        "unsaved-close-bar",
        json!([
            {"at_ms":1100,"target":"explorer-file:bar.rs"},
            {"at_ms":2700,"target":"workspace-close:bar.rs"}
        ]),
        3400,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st).iter().any(|s| {
                        s["kind"] == "editor"
                            && s["lifecycle"] == "running"
                            && s["label"] == "bar.rs"
                    })
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| {
                    s["kind"] == "editor" && s["lifecycle"] == "running" && s["label"] == "bar.rs"
                })
                .unwrap();
            h.write(&mut h.attach(editor)?, "iBar change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r.get("Text")
                                .and_then(Value::as_str)
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(&bar)? == "fn bar() {}\n",
        "Bar capture wrote the buffer"
    );
    h.assert_pids(&shells)
}
pub(crate) fn focus_close(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("focus-close")?;
    let path = h.root.join("focus-close/source.rs");
    fs::write(&path, "fn main() {}\n")?;
    let mut all = (0..5).map(|_| h.shell(&p)).collect::<Result<Vec<_>>>()?;
    let shells = all.clone();
    let editor = h.editor(&p, &path)?;
    all.push(editor.clone());
    h.layout(&p, &all)?;
    plain(
        &h,
        o,
        "strong-focus",
        json!([{"at_ms":1100,"target":format!("session-row:{}",id(shells.first().ok_or_else(|| anyhow!("missing shell"))?))}]),
        1500,
    )?;
    plain(
        &h,
        o,
        "editor-closed",
        json!([{"at_ms":1100,"target":format!("editor-close:{}",id(&editor))}]),
        3500,
    )?;
    h.wait(|st| session(st, id(&editor))["lifecycle"] == "ended", 5)?;
    ensure!(
        !session_ids(
            h.state()?
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
        )
        .contains(&id(&editor).into()),
        "Closed editor remains in layout"
    );
    h.assert_pids(&shells)
}
pub(crate) fn cleanup(o: &Options) -> Result<()> {
    let (h, _p, s, root) = setup("ui-cleanup")?;
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    let settings_object = settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?;
    settings_object.insert("external_editor".into(), json!("/bin/echo"));
    settings_object.insert("external_args".into(), json!(["one argument with spaces"]));
    settings_object.insert("notifications_side".into(), json!(false));
    h.rpc(json!({"Settings":settings}))?;
    fs::write(root.join("modified.rs"), "fn main() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "modified.rs"])?;
    fs::write(root.join("modified.rs"), "fn changed() {}\n")?;
    fs::create_dir(root.join("untracked-folder"))?;
    fs::write(root.join("untracked-folder/new file.rs"), "// new\n")?;
    fs::write(root.join("untracked.txt"), "new\n")?;
    plain(&h, o, "explorer", json!([]), 3500)?;
    ensure!(
        prefs(&h)?
            .get("attention_migrated")
            .is_some_and(|migrated| migrated.as_bool() == Some(true))
            && h.state()?
                .get("settings")
                .and_then(|settings| settings.get("notifications_side"))
                .is_some_and(|side| side.as_bool() == Some(true)),
        "Attention migration failed"
    );
    plain(
        &h,
        o,
        "overflow",
        Value::Array(
            (0u64..7)
                .map(|i| {
                    json!({"at_ms":1000u64.saturating_add(i.saturating_mul(450)),"target":"workspace-plus"})
                })
                .collect(),
        ),
        5300,
    )?;
    let state = h.state()?;
    let workspace = state
        .get("projects")
        .and_then(|projects| projects.get(0))
        .and_then(|project| project.get("layout"))
        .ok_or_else(|| anyhow!("missing projects[0].layout"))?;
    let tabs = workspace["tabs"].as_array().unwrap();
    ensure!(
        tabs.len() == 8
            && sessions(&state).len() == 13
            && workspace["active"] == tabs.last().unwrap()["id"],
        "Overflow + failed"
    );
    ensure!(session_ids(workspace).len() == 13, "Overflow lost panes");
    plain(
        &h,
        o,
        "editor-settings",
        json!([{"at_ms":900,"target":"tool-Git"},{"at_ms":1400,"target":"settings"},{"at_ms":2000,"target":"settings-section:Terminal & Editor"},{"at_ms":2300,"target":"external-program"},{"at_ms":2600,"target":"external-program","text":"/draft/editor with spaces"}]),
        4300,
    )?;
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("external_editor"))
            .is_some_and(|editor| editor == "/bin/echo")
            && h.state()?
                .get("settings")
                .and_then(|settings| settings.get("external_args"))
                .is_some_and(|args| args == &json!(["one argument with spaces"])),
        "Unsaved draft changed settings"
    );
    ensure!(
        prefs(&h)?.get("tool").is_some_and(|tool| tool == "Git"),
        "Settings changed sidebar tool"
    );
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert("notifications_side".into(), json!(false));
    h.rpc(json!({"Settings":settings}))?;
    plain(
        &h,
        o,
        "reopen-first-tab",
        json!([{"at_ms":1000,"target":format!("session-row:{}",id(s.first().ok_or_else(|| anyhow!("missing session"))?))}]),
        3500,
    )?;
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("notifications_side"))
            .is_some_and(|side| side.as_bool() == Some(false))
            && h.state()?
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .and_then(|layout| layout.get("active"))
                .is_some_and(|active| {
                    tabs.first()
                        .and_then(|tab| tab.get("id"))
                        .is_some_and(|id| active == id)
                }),
        "Restart lost user placement or selected workspace"
    );
    h.assert_pids(&s)
}
