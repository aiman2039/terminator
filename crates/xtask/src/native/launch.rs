//! Public sample workspaces only; no provider launch, user configuration or private messages.
use super::{Harness, Options, PathBuf, Result, fs, id, json, plain, prefs, save_prefs};

pub fn run(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.env
        .insert("TERMINATOR_TEST_SIZE".into(), "[1440,684]".into());
    h.setup()?;
    let web = h.project("atlas-web")?;
    let api = h.project("atlas-api")?;
    let root = PathBuf::from(web["path"].as_str().unwrap());
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
    plain(&h, o, "projects", json!([]), 2500)?;
    h.assert_pids(&[left.clone(), right.clone(), other])?;

    h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"launch-sample","terminal_session_id":id(&left),"agent_invocation_id":"launch-sample-agent","agent_kind":"sample","state":"waiting_input","request_id":"launch-sample-request","sequence":1,"summary":"Review the documentation changes","details":"Sample hook event for the launch gallery. No agent provider was launched.","resume":null}}))?;
    let mut preferences = prefs(&h)?;
    preferences["left_agents"] = json!(true);
    save_prefs(&h, &preferences)?;
    plain(&h, o, "attention", json!([]), 2300)?;

    let editor = h.editor(&web, &root.join("README.md"))?;
    h.layout(&web, std::slice::from_ref(&editor))?;
    preferences["left_agents"] = json!(false);
    save_prefs(&h, &preferences)?;
    plain(
        &h,
        o,
        "editing",
        json!([{"at_ms":900,"target":"markdown-mode:Split"}]),
        2600,
    )?;

    h.layout(&web, &[left.clone(), right.clone()])?;
    h.write(
        &mut h.attach(&right)?,
        "printf '\\nNew output while the GUI was closed.\\n'\n",
    )?;
    plain(&h, o, "reconnect", json!([]), 2400)?;
    h.assert_pids(&[left, right, editor])?;
    Ok(())
}
