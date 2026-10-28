//! Public sample workspaces only; no provider launch, user configuration or private messages.
use super::CaptureInput;
use super::{
    Harness, Options, Path, PathBuf, Result, Value, fs, id, json, plain, prefs, save_prefs,
};

pub fn run(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.env
        .insert("TERMINATOR_TEST_SIZE".into(), "[1440,684]".into());
    h.setup()?;
    let web = h.project("atlas-web")?;
    let api = h.project("atlas-api")?;
    let root = PathBuf::from(
        web.get("path")
            .and_then(super::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
    );
    write_samples(&root)?;
    let left = h.shell(&web)?;
    let right = h.shell(&web)?;
    let other = h.shell(&api)?;
    h.rpc(json!({"Rename":{"session":id(&left),"label":"Workspace"}}))?;
    h.rpc(json!({"Rename":{"session":id(&right),"label":"Review checklist"}}))?;
    h.layout(&web, &[left.clone(), right.clone()])?;
    h.layout(&api, std::slice::from_ref(&other))?;
    h.rpc(json!({"SelectProject":{"project":id(&web)}}))?;
    save_prefs(
        &h,
        &json!({"version":1,"typography_migrated":true,"attention_migrated":true}),
    )?;
    for (session, file) in [(&left, "workspace.txt"), (&right, "checks.txt")] {
        h.write(
            &mut h.attach(session)?,
            &format!("printf '\\033[2J\\033[H'; cat {file}\n"),
        )?;
    }
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "projects",
        actions: json!([]),
        after: 2500,
    })?;
    h.assert_pids(&[left.clone(), right.clone(), other])?;
    let mut preferences = attention(&h, o, &left)?;
    let editor = h.editor(&web, &root.join("README.md"))?;
    h.layout(&web, std::slice::from_ref(&editor))?;
    set_left_agents(&mut preferences, false)?;
    save_prefs(&h, &preferences)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "editing",
        actions: json!([{"at_ms":900,"target":"markdown-mode:Split"}]),
        after: 2600,
    })?;

    h.layout(&web, &[left.clone(), right.clone()])?;
    h.write(
        &mut h.attach(&right)?,
        "printf '\\nNew output while the GUI was closed.\\n'\n",
    )?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "reconnect",
        actions: json!([]),
        after: 2400,
    })?;
    h.assert_pids(&[left, right, editor])?;
    Ok(())
}

fn write_samples(root: &Path) -> Result<()> {
    fs::write(
        root.join("README.md"),
        "# Atlas workspace\n\nA small **sample project** for exploring a native workflow.\n\n## Today\n\n- [x] Organize project terminals\n- [x] Review documentation beside the source\n- [ ] Connect your own coding tools\n\n## Commands\n\n```sh\ncargo test\ncargo run\n```\n\n## Notes\n\nYour editor keeps its configuration.\nYour sessions keep running when the window closes.\n",
    )?;
    fs::write(
        root.join("workspace.txt"),
        "ATLAS / WEB\n\nSample project workspace\n\n  README.md        Project notes\n  workspace.txt    Terminal sample\n  checks.txt       Review checklist\n\nOne project. Your own tabs and splits.\n",
    )?;
    fs::write(
        root.join("checks.txt"),
        "REVIEW CHECKLIST\n\nSample terminal output\n\n  [x] Keep the project context together\n  [x] Open a second terminal beside the first\n  [x] Read project notes without leaving the workspace\n\nReady for your next command.\n",
    )?;
    Ok(())
}

fn set_left_agents(preferences: &mut Value, open: bool) -> Result<()> {
    preferences
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("preferences are not an object"))?
        .insert("left_agents".into(), json!(open));
    Ok(())
}

/// Report a sample waiting agent and capture the Agents view; returns the saved preferences.
fn attention(h: &Harness, o: &Options, left: &Value) -> Result<Value> {
    h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"launch-sample","terminal_session_id":id(left),"agent_invocation_id":"launch-sample-agent","agent_kind":"sample","state":"waiting_input","request_id":"launch-sample-request","sequence":1,"summary":"Review the documentation changes","details":"Sample hook event for the launch gallery. No agent provider was launched.","resume":null}}))?;
    let mut preferences = prefs(h)?;
    set_left_agents(&mut preferences, true)?;
    save_prefs(h, &preferences)?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "attention",
        actions: json!([]),
        after: 2300,
    })?;
    Ok(preferences)
}
