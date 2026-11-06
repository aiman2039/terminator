mod cleanup;
mod file_close;
mod focus_close;

use super::super::harness::{id, session, session_closed, session_ids, sessions};
use super::dispatch::CaptureInput;
use super::dispatch::{Options, capture, plain, setup};
use anyhow::{Result, anyhow, ensure};
pub(crate) use cleanup::cleanup;
pub(crate) use file_close::file_close;
pub(crate) use focus_close::focus_close;
use serde_json::json;
use std::fs;
pub(crate) fn inline_rename(o: &Options) -> Result<()> {
    let (h, _, s, _) = setup("inline-titles")?;
    let aux = s.get(1).ok_or_else(|| anyhow!("missing session"))?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "renamed",
        actions: json!([{"at_ms":1000,"target":"workspace-tab:Terminal 1","right_click":true},{"at_ms":1300,"target":"Rename terminal…"},{"at_ms":1700,"target":"rename-input","text":"Build workspace"},{"at_ms":2000,"target":"rename-input","key":"Enter"},{"at_ms":2500,"target":"terminal","right_click":true},{"at_ms":2800,"target":"Rename terminal…"},{"at_ms":3200,"target":"rename-input","text":"Worker pane"},{"at_ms":3500,"target":"rename-input","key":"Enter"},{"at_ms":4000,"target":format!("session-row:{}",id(aux)),"right_click":true},{"at_ms":4300,"target":"Rename terminal…"},{"at_ms":4700,"target":"rename-input","text":"Aux shell"},{"at_ms":5000,"target":"rename-input","key":"Enter"}]),
        after: 6400,
    })?;
    let state = h.state()?;
    for (index, label) in [(0, "Build workspace"), (5, "Worker pane"), (1, "Aux shell")] {
        let renamed = s.get(index).ok_or_else(|| anyhow!("missing session"))?;
        ensure!(
            session(&state, id(renamed))["label"] == label,
            "Rename targeted wrong session"
        );
    }
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "editing-inline",
        actions: json!([{"at_ms":900,"target":"workspace-tab:Build workspace","right_click":true},{"at_ms":1200,"target":"Rename terminal…"},{"at_ms":1500,"target":"rename-input","text":"Editing inline"}]),
        after: 2300,
    })?;
    h.assert_pids(&s)
}
/// IDE mode moves shells into the bottom strip. A rename from the terminal
/// menu must paint an editable field there.
pub(crate) fn ide_rename(o: &Options) -> Result<()> {
    let (h, _, s, _) = setup("ide-titles")?;
    let mut preferences = super::dispatch::prefs(&h)?;
    let fields = preferences
        .as_object_mut()
        .ok_or_else(|| anyhow!("preferences are not an object"))?;
    fields.insert("ide_mode".into(), json!(true));
    fields.insert("ide_sidebars_full_height".into(), json!(false));
    fields.insert("visible".into(), json!(false));
    super::dispatch::save_prefs(&h, &preferences)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "ide-renamed",
        actions: json!([{"at_ms":1500,"target":"ide-terminal-panel","hover_offset":[400.0,1.0],"hold":true},{"at_ms":1700,"target":"projects-sidebar","hover_offset":[150.0,40.0]},{"at_ms":1900,"target":"projects-sidebar","hover_offset":[150.0,40.0],"release_button":true},{"at_ms":2500,"target":"terminal","right_click":true},{"at_ms":2900,"target":"Rename terminal…"},{"at_ms":3400,"target":"rename-input","text":"Strip shell"},{"at_ms":4200,"target":"rename-input","key":"Enter"}]),
        after: 5200,
    })?;
    let state = h.state()?;
    ensure!(
        sessions(&state)
            .iter()
            .any(|session| session["label"] == "Strip shell"),
        "IDE strip rename did not save"
    );
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "ide-editing",
        actions: json!([{"at_ms":1500,"target":"terminal","right_click":true},{"at_ms":1900,"target":"Rename terminal…"},{"at_ms":2400,"target":"rename-input","text":"Editing strip"}]),
        after: 3200,
    })?;
    h.assert_pids(&s)
}
pub(crate) fn editor_lifecycle(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("editor-isolation")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    let shell = s.get(5).ok_or_else(|| anyhow!("missing session"))?;
    let actions = json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":5000,"target":format!("session-row:{}",id(shell)),"right_click":true},{"at_ms":5400,"target":"Rename terminal…"},{"at_ms":5900,"target":"rename-input","text":"Workspace shell"},{"at_ms":6400,"target":"rename-input","key":"Enter"},{"at_ms":6900,"target":format!("session-row:{}",id(shell))}]);
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "editor-lifecycle",
            actions,
            after: 8500,
        },
        |_| {
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
            h.wait(|s| session_closed(s, id(editor)), 5)?;
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
        },
    )?;
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
