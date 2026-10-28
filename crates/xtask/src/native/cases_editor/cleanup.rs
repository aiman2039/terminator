use super::super::super::harness::{
    Harness, first_layout, git, id, require_first_layout, session_ids, sessions,
};
use super::super::dispatch::{CaptureInput, Case, Options, plain, prefs, setup};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) fn cleanup(o: &Options) -> Result<()> {
    let (h, _p, s, root) = setup("ui-cleanup")?;
    seed_settings(&h)?;
    seed_git_tree(&root)?;
    explorer(&h, o)?;
    let overflow_state = overflow(&h, o)?;
    editor_settings_draft(&h, o)?;
    reopen_first_tab(Case { h: &h, o }, &s, &overflow_state)?;
    h.assert_pids(&s)
}

fn seed_settings(h: &Harness) -> Result<()> {
    h.update_settings(|settings| {
        settings.insert("external_editor".into(), json!("/bin/echo"));
        settings.insert("external_args".into(), json!(["one argument with spaces"]));
        settings.insert("notifications_side".into(), json!(false));
    })
}

fn seed_git_tree(root: &Path) -> Result<()> {
    fs::write(root.join("modified.rs"), "fn main() {}\n")?;
    git(root, &["init", "-q"])?;
    git(root, &["add", "modified.rs"])?;
    fs::write(root.join("modified.rs"), "fn changed() {}\n")?;
    fs::create_dir(root.join("untracked-folder"))?;
    fs::write(root.join("untracked-folder/new file.rs"), "// new\n")?;
    fs::write(root.join("untracked.txt"), "new\n")?;
    Ok(())
}

fn explorer(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "explorer",
        actions: json!([]),
        after: 3500,
    })?;
    ensure!(
        prefs(h)?
            .get("attention_migrated")
            .is_some_and(|migrated| migrated.as_bool() == Some(true))
            && h.state()?
                .get("settings")
                .and_then(|settings| settings.get("notifications_side"))
                .is_some_and(|side| side.as_bool() == Some(true)),
        "Attention migration failed"
    );
    Ok(())
}

/// Open seven more workspace tabs; return the snapshot holding the new tabs.
fn overflow(h: &Harness, o: &Options) -> Result<Value> {
    plain(CaptureInput { h, opts: o, name: "overflow", actions: Value::Array(
            (0u64..7)
                .map(|i| {
                    json!({"at_ms":1000u64.saturating_add(i.saturating_mul(450)),"target":"workspace-plus"})
                })
                .collect(),
        ), after: 5300 })?;
    let state = h.state()?;
    let workspace = require_first_layout(&state)?;
    let tabs = workspace["tabs"].as_array().unwrap();
    ensure!(
        tabs.len() == 8
            && sessions(&state).len() == 13
            && workspace["active"] == tabs.last().unwrap()["id"],
        "Overflow + failed"
    );
    ensure!(session_ids(workspace).len() == 13, "Overflow lost panes");
    Ok(state)
}

fn editor_settings_draft(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "editor-settings",
        actions: json!([{"at_ms":900,"target":"tool-Git"},{"at_ms":1200,"target":"header-overflow"},{"at_ms":1400,"target":"settings"},{"at_ms":2000,"target":"settings-section:Terminal & Editor"},{"at_ms":2300,"target":"external-program"},{"at_ms":2600,"target":"external-program","text":"/draft/editor with spaces"}]),
        after: 4300,
    })?;
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
        prefs(h)?.get("tool").is_some_and(|tool| tool == "git"),
        "Settings changed sidebar tool"
    );
    Ok(())
}

fn reopen_first_tab(case: Case<'_>, s: &[Value], overflow_state: &Value) -> Result<()> {
    let Case { h, o } = case;
    let tabs = require_first_layout(overflow_state)?["tabs"]
        .as_array()
        .unwrap();
    h.set_setting("notifications_side", json!(false))?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "reopen-first-tab",
        actions: json!([{"at_ms":1000,"target":format!("session-row:{}",id(s.first().ok_or_else(|| anyhow!("missing session"))?))}]),
        after: 3500,
    })?;
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("notifications_side"))
            .is_some_and(|side| side.as_bool() == Some(false))
            && first_layout(&h.state()?)
                .and_then(|layout| layout.get("active"))
                .is_some_and(|active| {
                    tabs.first()
                        .and_then(|tab| tab.get("id"))
                        .is_some_and(|id| active == id)
                }),
        "Restart lost user placement or selected workspace"
    );
    Ok(())
}
