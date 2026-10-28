use super::super::super::harness::{
    Harness, first_layout, id, require_first_layout, session_closed, session_ids, sessions,
};
use super::super::dispatch::{CaptureInput, Options, plain, setup};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::fs;

/// The editor opened from the explorer and the two top-level tabs it splits into.
struct Groups<'a> {
    editor: &'a Value,
    shell: &'a Value,
    editor_tab: &'a Value,
}

pub(crate) fn workspace_tabs(o: &Options) -> Result<()> {
    let (h, _p, originals, root) = setup("workspace-tabs")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    let state = capture_independent_editor(&h, o)?;
    let groups = Groups::find(&state, &originals)?;
    pin_and_unpin(&h, o, groups.editor_tab)?;
    restore_shell_layout(&h, o, groups.shell)?;
    close_clean_editor(&h, o, groups.editor)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "shell-close-dialog",
        actions: json!([{"at_ms":1100,"target":"workspace-close:Terminal 1"}]),
        after: 3000,
    })?;
    h.assert_pids(&originals)
}

fn capture_independent_editor(h: &Harness, o: &Options) -> Result<Value> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "independent-editor",
        actions: json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2100,"target":"workspace-strip","scroll":500.0},{"at_ms":2500,"target":"workspace-tab:Terminal 1"},{"at_ms":3200,"target":"terminal","right_click":true},{"at_ms":3700,"target":"Split right"},{"at_ms":4200,"target":"workspace-strip","scroll":-500.0},{"at_ms":4600,"target":"workspace-tab:source.rs"}]),
        after: 6500,
    })?;
    h.state()
}

impl<'a> Groups<'a> {
    /// Locate the editor and both tabs, checking the split kept its owners.
    fn find(state: &'a Value, originals: &[Value]) -> Result<Self> {
        let editor = sessions(state)
            .iter()
            .find(|s| s["kind"] == "editor")
            .context("Editor not created")?;
        let layout = require_first_layout(state)?;
        ensure!(
            layout["tabs"].as_array().unwrap().len() == 2,
            "File did not open top-level tab"
        );
        let original = originals
            .first()
            .ok_or_else(|| anyhow!("missing original session"))?;
        let tab_holding = |session: &Value| {
            layout["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| session_ids(&t["layout"]).contains(&id(session).into()))
                .unwrap()
        };
        let shell = tab_holding(original);
        let editor_tab = tab_holding(editor);
        ensure!(
            session_ids(&shell["layout"]).len() == 7
                && session_ids(&editor_tab["layout"]) == [id(editor)],
            "Split ownership changed"
        );
        ensure!(layout["active"] == editor_tab["id"], "Editor not selected");
        Ok(Self {
            editor,
            shell,
            editor_tab,
        })
    }
}

fn pin_and_unpin(h: &Harness, o: &Options, editor_tab: &Value) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "pinned-editor",
        actions: json!([
            {"at_ms": 600, "target": "workspace-tab:source.rs", "right_click": true},
            {"at_ms": 1050, "target": "Pin tab"}
        ]),
        after: 2100,
    })?;
    ensure!(
        h.state()?
            .get("projects")
            .and_then(serde_json::Value::as_array)
            .and_then(|projects| projects.first())
            .and_then(|project| project.get("layout"))
            .and_then(|layout| layout.get("tabs"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tabs| tabs
                .iter()
                .any(|tab| { tab["id"] == editor_tab["id"] && tab["pinned"] == true })),
        "Editor tab pin did not persist"
    );
    plain(CaptureInput {
        h,
        opts: o,
        name: "unpinned-editor",
        actions: json!([
            {"at_ms": 600, "target": "workspace-tab:source.rs", "right_click": true},
            {"at_ms": 1050, "target": "Unpin tab"}
        ]),
        after: 2100,
    })?;
    Ok(())
}

fn restore_shell_layout(h: &Harness, o: &Options, shell: &Value) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "restored-shell-layout",
        actions: json!([{"at_ms":700,"target":if o.narrow {"tabs-left"} else {"workspace-tab:Terminal 1"}},{"at_ms":1100,"target":"workspace-tab:Terminal 1"}]),
        after: 3500,
    })?;
    ensure!(
        first_layout(&h.state()?)
            .and_then(|layout| layout.get("active"))
            .is_some_and(|active| active == &shell["id"]),
        "Selected tab not restored"
    );
    Ok(())
}

fn close_clean_editor(h: &Harness, o: &Options, editor: &Value) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "clean-editor-close",
        actions: if o.narrow {
            json!([
                {"at_ms":800,"target":"tabs-right"},
                {"at_ms":1600,"target":"workspace-close:source.rs"}
            ])
        } else {
            json!([{"at_ms":1100,"target":"workspace-close:source.rs"}])
        },
        after: 3500,
    })?;
    h.wait(|s| session_closed(s, id(editor)), 5)?;
    ensure!(
        session_ids(require_first_layout(&h.state()?)?).len() == 7,
        "Closing editor removed shell"
    );
    Ok(())
}
