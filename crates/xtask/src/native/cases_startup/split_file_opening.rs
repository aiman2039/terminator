use super::super::super::harness::{
    Harness, id, require_first_layout, session, session_ids, sessions,
};
use super::super::dispatch::{CaptureInput, Options, plain};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

/// One shell in a project whose directory was moved after it started.
struct Fixture {
    h: Harness,
    project: Value,
    original: Value,
    old_root: PathBuf,
    moved_root: PathBuf,
}

pub(crate) fn split_file_opening(o: &Options) -> Result<()> {
    let f = launch()?;
    refuse_unavailable_directory(&f)?;
    // A compatibility path lets the existing daemon, shells and saved layouts
    // survive a move without restarting sessions or rewriting their identities.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&f.moved_root, &f.old_root)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&f.moved_root, &f.old_root)?;
    for split in [
        ("up", "Vertical", 1),
        ("down", "Vertical", 2),
        ("left", "Horizontal", 1),
        ("right", "Horizontal", 2),
    ] {
        split_terminal(&f, o, split)?;
    }
    for open in [
        ("clicked.rs", None, false),
        ("menu.rs", Some("Open file"), false),
        ("split.rs", Some("Open in editor split"), true),
    ] {
        open_file(&f, o, open)?;
    }
    Ok(())
}

fn launch() -> Result<Fixture> {
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
    Ok(Fixture {
        h,
        project,
        original,
        old_root,
        moved_root,
    })
}

fn refuse_unavailable_directory(f: &Fixture) -> Result<()> {
    let Fixture {
        h,
        project,
        original,
        old_root,
        ..
    } = f;
    for editor in [false, true] {
        let error = h
            .rpc(json!({"Create":{
                "project":id(project),"cwd":old_root,
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
    h.assert_pids(std::slice::from_ref(original))
}

fn split_terminal(
    f: &Fixture,
    o: &Options,
    (direction, axis, new_index): (&str, &str, usize),
) -> Result<()> {
    let Fixture {
        h,
        project,
        original,
        moved_root,
        ..
    } = f;
    h.layout(project, std::slice::from_ref(original))?;
    let before = sessions(&h.state()?).len();
    plain(CaptureInput {
        h,
        opts: o,
        name: &format!("split-{direction}"),
        actions: json!([
            {"at_ms":1000,"target":"terminal","right_click":true},
            {"at_ms":1700,"target":format!("Split {direction}")}
        ]),
        after: 3000,
    })?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == before.saturating_add(1),
        "Split {direction} did not create exactly one shell"
    );
    let layout = require_first_layout(&state)?;
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
        session_ids(&nodes[3usize.saturating_sub(new_index)]) == [id(original)],
        "Split {direction} moved the original to the wrong side"
    );
    let created_ids = session_ids(&nodes[new_index]);
    ensure!(
        created_ids.len() == 1
            && created_ids
                .first()
                .is_some_and(|created| created != id(original)),
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
    Ok(())
}

fn open_file(
    f: &Fixture,
    o: &Options,
    (name, menu, split): (&str, Option<&str>, bool),
) -> Result<()> {
    let Fixture {
        h,
        project,
        original,
        ..
    } = f;
    h.layout(project, std::slice::from_ref(original))?;
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
    plain(CaptureInput {
        h,
        opts: o,
        name,
        actions: json!(actions),
        after: 3300,
    })?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == before.saturating_add(1),
        "Opening {name} did not create exactly one editor"
    );
    check_opened_editor(f, &state, (name, split))?;
    h.assert_pids(std::slice::from_ref(original))
}

fn check_opened_editor(f: &Fixture, state: &Value, (name, split): (&str, bool)) -> Result<()> {
    let Fixture {
        old_root,
        moved_root,
        ..
    } = f;
    let editor = sessions(state)
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
    let layout = require_first_layout(state)?;
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
    Ok(())
}
