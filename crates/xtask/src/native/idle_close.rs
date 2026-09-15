use super::*;

pub fn run(o: &Options) -> Result<()> {
    let Some(shell) = terminator_core::find_executable("zsh") else {
        anyhow::bail!("Native idle-close fixture requires zsh");
    };
    let mut h = Harness::new()?;
    let home = h.root.join("home");
    fs::create_dir_all(&home)?;
    h.env
        .insert("HOME".into(), home.to_string_lossy().into_owned());
    h.env
        .insert("ZDOTDIR".into(), home.to_string_lossy().into_owned());
    h.restart()?;
    let mut settings = h.state()?["settings"].clone();
    settings["shell"] = json!(shell);
    h.rpc(json!({"Settings":settings}))?;
    let project = h.project("idle-close-native")?;
    let idle = h.shell(&project)?;
    let busy = h.shell(&project)?;
    h.layout(&project, &[idle.clone(), busy.clone()])?;
    thread::sleep(Duration::from_millis(400));
    h.write(&mut h.attach(&busy)?, "sleep 30\r")?;
    plain(
        &h,
        o,
        "idle-pane-closed",
        json!([{"at_ms":1300,"target":format!("pane-close:{}", id(&idle))}]),
        3600,
    )?;
    let state = h.state()?;
    ensure!(
        !matches!(
            session(&state, id(&idle))["lifecycle"].as_str(),
            Some("running" | "stopping")
        ),
        "Idle pane still requires confirmation"
    );
    h.assert_pids(std::slice::from_ref(&busy))?;
    ensure!(
        !session_ids(&state["projects"][0]["layout"]).contains(&id(&idle).to_owned()),
        "Closed idle pane remains in layout"
    );
    Ok(())
}
