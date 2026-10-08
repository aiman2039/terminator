use super::super::harness::{Harness, id, session, session_ids, sessions};
use super::dispatch::{Options, plain, prefs, save_prefs, setup};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::json;
use std::{fs, path::PathBuf};
pub(crate) fn smoke(opts: &Options) -> Result<()> {
    ensure!(
        (6..=100).contains(&opts.sessions),
        "Smoke requires 6–100 sessions"
    );
    let mut h = Harness::new()?;
    h.setup()?;
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert("font_size".into(), json!(16.0));
    h.rpc(json!({"Settings":settings}))?;
    let p = h.project("native-ui")?;
    let path = h.root.join("native-ui/hello.rs");
    fs::write(
        &path,
        "fn main() {\n    println!(\"Native Rust terminal\");\n}\n",
    )?;
    let mut originals = Vec::new();
    for i in 0usize..5 {
        let s = h.shell(&p)?;
        h.write(
            &mut h.attach(&s)?,
            &format!(
                "printf '\\033[1;36mSESSION {}\\033[0m\\nNative terminal is connected.\\n'\n",
                i.saturating_add(1)
            ),
        )?;
        originals.push(s);
    }
    let editor = h.editor(&p, &path)?;
    originals.push(editor.clone());
    for _ in 6..opts.sessions {
        h.shell(&p)?;
    }
    h.layout(&p, &originals)?;
    h.env.insert("TERMINATOR_TEST_INPUT".into(), "1".into());
    plain(
        &h,
        opts,
        "six-panes",
        json!([]),
        opts.seconds.saturating_mul(1000),
    )?;
    h.env.remove("TERMINATOR_TEST_INPUT");
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("font_size"))
            .is_some_and(|size| size == &json!(13.0)),
        "Typography migration did not apply"
    );
    let mut pref = prefs(&h)?;
    ensure!(
        pref.get("typography_migrated")
            .is_some_and(|migrated| migrated == &json!(true)),
        "Migration marker missing"
    );
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert("font_size".into(), json!(18.0));
    h.rpc(json!({"Settings":settings}))?;
    let preferences = pref
        .as_object_mut()
        .ok_or_else(|| anyhow!("preferences are not an object"))?;
    preferences.insert("tool".into(), json!("git"));
    preferences.insert("visible".into(), json!(false));
    preferences.insert("width".into(), json!(370.0));
    preferences.insert("expanded".into(), json!({id(&p):false}));
    save_prefs(&h, &pref)?;
    plain(
        &h,
        opts,
        "restart",
        json!([]),
        opts.seconds.saturating_mul(1000),
    )?;
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("font_size"))
            .is_some_and(|size| size == &json!(18.0))
            && prefs(&h)? == pref,
        "User preferences changed on restart"
    );
    h.assert_pids(&originals)?;
    ensure!(
        session_ids(
            h.state()?
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
        )
        .len()
            == 6,
        "Layout lost panes"
    );
    ensure!(
        h.history(id(&editor))?.contains("fn main()")
            && fs::read_to_string(path)?.contains("UI_INSERT"),
        "Native editor input/save failed"
    );
    Ok(())
}
pub(crate) fn workspace_tabs(o: &Options) -> Result<()> {
    let (h, _p, originals, root) = setup("workspace-tabs")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    plain(
        &h,
        o,
        "independent-editor",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2100,"target":"workspace-strip","scroll":500.0},{"at_ms":2500,"target":"workspace-tab:Terminal 1"},{"at_ms":3200,"target":"terminal","right_click":true},{"at_ms":3700,"target":"Split right"},{"at_ms":4200,"target":"workspace-strip","scroll":-500.0},{"at_ms":4600,"target":"workspace-tab:source.rs"}]),
        6500,
    )?;
    let state = h.state()?;
    let editor = sessions(&state)
        .iter()
        .find(|s| s["kind"] == "editor")
        .context("Editor not created")?;
    let layout = state
        .get("projects")
        .and_then(|projects| projects.get(0))
        .and_then(|project| project.get("layout"))
        .ok_or_else(|| anyhow!("missing projects[0].layout"))?;
    ensure!(
        layout["tabs"].as_array().unwrap().len() == 2,
        "File did not open top-level tab"
    );
    let original = originals
        .first()
        .ok_or_else(|| anyhow!("missing original session"))?;
    let shell_group = layout["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| session_ids(&t["layout"]).contains(&id(original).into()))
        .unwrap();
    let editor_group = layout["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| session_ids(&t["layout"]).contains(&id(editor).into()))
        .unwrap();
    ensure!(
        session_ids(&shell_group["layout"]).len() == 7
            && session_ids(&editor_group["layout"]) == [id(editor)],
        "Split ownership changed"
    );
    ensure!(
        layout["active"] == editor_group["id"],
        "Editor not selected"
    );
    plain(
        &h,
        o,
        "restored-shell-layout",
        json!([{"at_ms":700,"target":if o.narrow {"tabs-left"} else {"workspace-tab:Terminal 1"}},{"at_ms":1100,"target":"workspace-tab:Terminal 1"}]),
        3500,
    )?;
    ensure!(
        h.state()?
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .and_then(|layout| layout.get("active"))
            .is_some_and(|active| active == &shell_group["id"]),
        "Selected tab not restored"
    );
    plain(
        &h,
        o,
        "clean-editor-close",
        if o.narrow {
            json!([
                {"at_ms":800,"target":"tabs-right"},
                {"at_ms":1600,"target":"workspace-close:source.rs"}
            ])
        } else {
            json!([{"at_ms":1100,"target":"workspace-close:source.rs"}])
        },
        3500,
    )?;
    h.wait(|s| session(s, id(editor))["lifecycle"] == "ended", 5)?;
    ensure!(
        session_ids(
            h.state()?
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
        )
        .len()
            == 7,
        "Closing editor removed shell"
    );
    plain(
        &h,
        o,
        "shell-close-dialog",
        json!([{"at_ms":1100,"target":"workspace-close:Terminal 1"}]),
        3000,
    )?;
    h.assert_pids(&originals)
}
pub(crate) fn split_file_opening(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("original-project")?;
    let original = h.shell(&project)?;
    let old_root = PathBuf::from(
        project
            .get("path")
            .ok_or_else(|| anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    for name in ["clicked.rs", "menu.rs", "split.rs"] {
        fs::write(old_root.join(name), "fn main() {}\n")?;
    }
    let moved_root = h.root.join("moved-project");
    fs::rename(&old_root, &moved_root)?;
    for editor in [false, true] {
        let error = h
            .rpc(json!({"Create":{
                "project":id(&project),"cwd":old_root,
                "file":editor.then(|| old_root.join("clicked.rs")),
                "editor":editor
            }}))
            .expect_err("An unavailable directory must not create a session");
        let message = format!("{error:#}");
        ensure!(
            message.contains(&old_root.to_string_lossy().to_string()) && message.contains("moved"),
            "Creation error must identify the missing directory and recovery: {message}"
        );
    }
    ensure!(
        sessions(&h.state()?).len() == 1,
        "Failed creates changed inventory"
    );
    h.assert_pids(std::slice::from_ref(&original))?;

    // A compatibility path lets the existing daemon, shells and saved layouts
    // survive a move without restarting sessions or rewriting their identities.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&moved_root, &old_root)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&moved_root, &old_root)?;
    for (direction, axis, new_index) in [
        ("up", "Vertical", 1),
        ("down", "Vertical", 2),
        ("left", "Horizontal", 1),
        ("right", "Horizontal", 2),
    ] {
        h.layout(&project, std::slice::from_ref(&original))?;
        let before = sessions(&h.state()?).len();
        plain(
            &h,
            o,
            &format!("split-{direction}"),
            json!([
                {"at_ms":1000,"target":"terminal","right_click":true},
                {"at_ms":1700,"target":format!("Split {direction}")}
            ]),
            3000,
        )?;
        let state = h.state()?;
        ensure!(
            sessions(&state).len() == before.saturating_add(1),
            "Split {direction} did not create exactly one shell"
        );
        let layout = state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .ok_or_else(|| anyhow!("missing projects[0].layout"))?;
        ensure!(
            layout["tabs"].as_array().unwrap().len() == 1,
            "Split created a top-level tab"
        );
        let nodes = layout
            .get("tabs")
            .and_then(|tabs| tabs.get(0))
            .and_then(|tab| tab.get("layout"))
            .and_then(|layout| layout.get("surfaces"))
            .and_then(|surfaces| surfaces.get(0))
            .and_then(|surface| surface.get("Main"))
            .and_then(|main| main.get("nodes"))
            .ok_or_else(|| anyhow!("missing split nodes"))?;
        ensure!(
            nodes[0].get(axis).is_some(),
            "Wrong split orientation: {direction}"
        );
        ensure!(
            session_ids(&nodes[3usize.saturating_sub(new_index)]) == [id(&original)],
            "Split {direction} moved the original to the wrong side"
        );
        let created_ids = session_ids(&nodes[new_index]);
        ensure!(
            created_ids.len() == 1
                && created_ids
                    .first()
                    .is_some_and(|created| created != id(&original)),
            "New split is missing"
        );
        ensure!(
            session(
                &state,
                created_ids
                    .first()
                    .ok_or_else(|| anyhow!("missing split session"))?,
            )["cwd"]
                == moved_root.to_string_lossy().as_ref(),
            "Split did not resolve relocated directory"
        );
    }

    for (name, menu, split) in [
        ("clicked.rs", None, false),
        ("menu.rs", Some("Open file"), false),
        ("split.rs", Some("Open in editor split"), true),
    ] {
        h.layout(&project, std::slice::from_ref(&original))?;
        let before = sessions(&h.state()?).len();
        let mut actions = vec![json!({
            "at_ms":1100,"target":format!("explorer-file:{name}"),"right_click":menu.is_some()
        })];
        if let Some(menu) = menu {
            actions.push(json!({"at_ms":1800,"target":menu}));
        } else {
            // A double-click must create one editor, with no duplicate swap-file prompt.
            actions.push(json!({"at_ms":1200,"target":format!("explorer-file:{name}")}));
        }
        plain(&h, o, name, json!(actions), 3300)?;
        let state = h.state()?;
        ensure!(
            sessions(&state).len() == before.saturating_add(1),
            "Opening {name} did not create exactly one editor"
        );
        let editor = sessions(&state)
            .iter()
            .find(|s| s["file"] == old_root.join(name).to_string_lossy().as_ref())
            .context("Opened file is missing")?;
        ensure!(
            editor["kind"] == "editor" && editor["lifecycle"] == "running",
            "File editor exited"
        );
        ensure!(
            editor["cwd"] == moved_root.to_string_lossy().as_ref(),
            "Editor did not resolve relocated directory"
        );
        let layout = state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .ok_or_else(|| anyhow!("missing projects[0].layout"))?;
        let tabs = layout["tabs"].as_array().unwrap();
        ensure!(
            tabs.len() == if split { 1 } else { 2 },
            "Wrong file tab placement"
        );
        let active = tabs
            .iter()
            .find(|t| t["id"] == layout["active"])
            .context("Active tab missing")?;
        let ids = session_ids(&active["layout"]);
        ensure!(
            ids.contains(&id(editor).to_owned()) && ids.len() == if split { 2 } else { 1 },
            "Opened editor is not visible in its target tab"
        );
        h.assert_pids(std::slice::from_ref(&original))?;
    }
    Ok(())
}
