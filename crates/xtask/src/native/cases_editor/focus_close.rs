use super::super::super::harness::{
    Harness, bin, id, require_first_layout, session, session_closed, session_ids, sessions,
};
use super::super::dispatch::{CaptureInput, Options, capture, plain};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};

/// Five shells and one editor laid out in the `focus-close` project.
struct Fixture {
    h: Harness,
    p: Value,
    path: std::path::PathBuf,
    shells: Vec<Value>,
    editor: Value,
}

pub(crate) fn focus_close(o: &Options) -> Result<()> {
    let f = launch()?;
    close_focused_editor(&f, o)?;
    f.h.assert_pids(&f.shells)?;
    save_origin_tabs(&f)?;
    leftmost_file_close(&f, o)?;
    new_tab_after_cd(&f, o)?;
    f.h.assert_pids(&f.shells)
}

fn launch() -> Result<Fixture> {
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
    Ok(Fixture {
        h,
        p,
        path,
        shells,
        editor,
    })
}

fn close_focused_editor(f: &Fixture, o: &Options) -> Result<()> {
    let Fixture {
        h, shells, editor, ..
    } = f;
    plain(CaptureInput {
        h,
        opts: o,
        name: "strong-focus",
        actions: json!([{"at_ms":1100,"target":format!("session-row:{}",id(shells.first().ok_or_else(|| anyhow!("missing shell"))?))}]),
        after: 1500,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "editor-closed",
        actions: json!([{"at_ms":1100,"target":format!("editor-close:{}",id(editor))}]),
        after: 3500,
    })?;
    h.wait(|st| session_closed(st, id(editor)), 5)?;
    ensure!(
        !session_ids(require_first_layout(&h.state()?)?).contains(&id(editor).into()),
        "Closed editor remains in layout"
    );
    Ok(())
}

fn save_origin_tabs(f: &Fixture) -> Result<()> {
    let Fixture { h, p, shells, .. } = f;
    let tabs = shells
        .iter()
        .take(3)
        .enumerate()
        .map(|(index, shell)| {
            Ok(json!({
                "id":format!("origin-{index}"),
                "primary":{"Terminal":id(shell)},
                "layout":h.layout(p, std::slice::from_ref(shell))?
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    h.rpc(json!({"SaveLayout":{"project":id(p),"layout":{"version":2,"active":"origin-0","tabs":tabs}}}))?;
    Ok(())
}

fn leftmost_file_close(f: &Fixture, o: &Options) -> Result<()> {
    let Fixture { h, p, path, .. } = f;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "leftmost-file-close",
            actions: json!([
                {"at_ms":1100,"target":"explorer-file:source.rs"},
                {"at_ms":2700,"target":"workspace-close:source.rs"}
            ]),
            after: 4200,
        },
        |_| wait_running_file(h, path),
    )?;
    let state = h.state()?;
    let layout = state
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|project| id(project) == id(p))
        .and_then(|project| project.get("layout"))
        .ok_or_else(|| anyhow!("Project layout missing"))?;
    ensure!(
        layout
            .get("active")
            .is_some_and(|active| active == "origin-0"),
        "File close did not restore the far-left tab: {layout}"
    );
    ensure!(
        layout
            .get("tabs")
            .and_then(Value::as_array)
            .is_some_and(|tabs| tabs.len() == 3),
        "File tab remained after close"
    );
    Ok(())
}

fn wait_running_file(h: &Harness, path: &Path) -> Result<()> {
    h.wait(
        |state| {
            sessions(state).iter().any(|session| {
                session.get("kind").is_some_and(|kind| kind == "editor")
                    && session
                        .get("lifecycle")
                        .is_some_and(|lifecycle| lifecycle == "running")
                    && session
                        .get("file")
                        .is_some_and(|file| file == path.to_string_lossy().as_ref())
            })
        },
        3,
    )?;
    Ok(())
}

fn new_tab_after_cd(f: &Fixture, o: &Options) -> Result<()> {
    let Fixture { h, p, shells, .. } = f;
    let source = shells
        .first()
        .ok_or_else(|| anyhow!("Original shell missing"))?;
    let subdir = h.root.join("focus-close/subdir");
    fs::create_dir(&subdir)?;
    let subdir = subdir.canonicalize()?;
    h.write(
        &mut h.attach(source)?,
        &format!(
            "cd {}; {} cwd \"$PWD\"\n",
            terminator_core::quote(&subdir.to_string_lossy()),
            terminator_core::quote(&bin().join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(
        |state| {
            session(state, id(source))
                .get("cwd")
                .is_some_and(|cwd| cwd == subdir.to_string_lossy().as_ref())
        },
        5,
    )?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "new-tab-after-cd",
        actions: json!([{"at_ms":1100,"target":"workspace-plus"}]),
        after: 3200,
    })?;
    let state = h.state()?;
    let new_shell = sessions(&state)
        .iter()
        .find(|session| {
            session.get("kind").is_some_and(|kind| kind == "shell")
                && !shells.iter().any(|old| id(old) == id(session))
        })
        .ok_or_else(|| anyhow!("New workspace tab did not create a shell"))?;
    ensure!(
        new_shell.get("cwd") == p.get("path"),
        "New GUI tab inherited the shell directory"
    );
    ensure!(
        new_shell
            .get("project_id")
            .is_some_and(|project| project == id(p)),
        "New GUI tab changed project identity"
    );
    Ok(())
}
