//! Explicitly opt-in live provider fixture. Uses only newly generated public sample text.
use super::*;
use terminator_core::{Paths, ui_control};

fn snapshot(h: &Harness, sid: &str) -> Result<Value> {
    Ok(ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)?["terminal_scroll"][sid].clone())
}
fn attach(h: &Harness, session: &Value) -> Result<std::os::unix::net::UnixStream> {
    let state = h.state()?;
    let record = crate::harness::session(&state, id(session));
    let mut stream = h.connect(
        json!({"Attach":{"session":id(session),"rows":record["rows"],"cols":record["cols"]}}),
        None,
        None,
    )?;
    let _: Value = terminator_core::read_frame(&mut stream)?;
    Ok(stream)
}
fn markers(text: &str, prefix: &str, count: usize) -> Vec<usize> {
    (1..=count)
        .filter(|n| text.contains(&format!("{prefix}{n:03}")))
        .collect()
}
fn action(h: &Harness, actions: &mut Vec<Value>, value: Value) -> Result<()> {
    let mut value = value;
    value["capture"] = json!(true);
    actions.push(value);
    terminator_core::atomic_write(&h.root.join("actions.json"), &serde_json::to_vec(actions)?)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state = ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)?;
        if state["fixture_actions_completed"].as_u64() == Some(actions.len() as u64) {
            break;
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Native input was not delivered"
        );
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(180));
    Ok(())
}
fn gui(h: &Harness, output: &Path, mode: &str) -> Result<Process> {
    let log = fs::File::create(output.join(format!("{mode}-gui.log")))?;
    let mut command = h.command("terminator");
    command
        .env(
            "TERMINATOR_CAPTURE_PATH",
            output.join(format!("{mode}.png")),
        )
        .env("TERMINATOR_CAPTURE_AFTER_MS", "600000")
        .env("TERMINATOR_TEST_BACKGROUND", "1")
        .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1")
        .env("TERMINATOR_TEST_KEEP_OPEN", "1")
        .env("TERMINATOR_TEST_SIZE", "[1440,800]")
        .env("TERMINATOR_TEST_ACTIONS_PATH", h.root.join("actions.json"))
        .stdout(log.try_clone()?)
        .stderr(log);
    Ok(Process(command.spawn()?))
}
pub fn run(o: &Options) -> Result<()> {
    let codex = terminator_core::find_executable("codex").context("Codex is not installed")?;
    fs::create_dir_all(&o.output)?;
    let focus_only = std::env::var_os("TERMINATOR_TEST_CODEX_FOCUS_ONLY").is_some();
    for (mode, flag) in [("normal", ""), ("no-alt-screen", "--no-alt-screen")] {
        let run_label = format!("{mode}{}", if focus_only { "-focused" } else { "" });
        let evidence_path = o.output.join(format!("{run_label}-evidence.json"));
        let scratch = tempfile::Builder::new()
            .prefix(&format!("codex-scroll-{mode}-"))
            .tempdir_in(root().join("target"))?;
        let h = Harness::new()?;
        h.setup()?;
        h.rpc(json!({"AddProject":{"path":scratch.path()}}))?;
        let project = h.state()?["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == scratch.path().to_string_lossy().as_ref())
            .unwrap()
            .clone();
        let idle = h.shell(&project)?;
        let session = h.shell(&project)?;
        h.layout(&project, &[idle.clone(), session.clone()])?;
        fs::write(h.root.join("actions.json"), "[]")?;
        let mut native = gui(&h, &o.output, &run_label)?;
        h.wait(
            |_| snapshot(&h, id(&session)).is_ok_and(|s| s["rect"].is_array()),
            15,
        )?;
        thread::sleep(Duration::from_secs(1));
        let mut preflight = Vec::new();
        action(
            &h,
            &mut preflight,
            json!({"at_ms":0,"target":format!("terminal:{}",id(&idle)),"hover":true}),
        )?;
        action(
            &h,
            &mut preflight,
            json!({"at_ms":0,"target":format!("terminal:{}",id(&idle))}),
        )?;
        let preflight_state = snapshot(&h, id(&session))?;
        ensure!(
            preflight_state["focused"] == false,
            "Fixture cannot deliver a focus click before Codex launch: {preflight_state}"
        );
        // Retain action numbering for the same GUI input stream.
        let prompt = "This is a terminal scrolling validation with public sample data. Do not use tools, read files, or change anything. Output one fenced text block containing exactly 160 lines. Each line starts with TSAMPLE immediately followed by its three-digit line number (1 through 160), then a space and the words public sample. No commentary outside the block.";
        let command = format!(
            "{} --sandbox read-only --ask-for-approval never --disable hooks {flag} {}\r",
            terminator_core::quote(&codex.to_string_lossy()),
            terminator_core::quote(prompt)
        );
        let mut input = attach(&h, &session)?;
        h.write(&mut input, &command)?;
        println!(
            "Live Codex {mode}: started in disposable directory under the already-trusted repository"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        while !h.history(id(&session))?.contains("TSAMPLE160") {
            ensure!(
                std::time::Instant::now() < deadline,
                "Live Codex did not produce the sample response in {mode}"
            );
            thread::sleep(Duration::from_millis(250));
        }
        println!("Live Codex {mode}: sample response generated");
        thread::sleep(Duration::from_secs(1));
        let mut actions = preflight;
        let target = format!("terminal:{}", id(&session));
        let mut observations = Vec::new();
        action(
            &h,
            &mut actions,
            json!({"at_ms":0,"target":target,"hover":true}),
        )?;
        observations.push(json!({"phase":"latest","state":snapshot(&h,id(&session))?}));
        action(&h, &mut actions, json!({"at_ms":0,"target":target}))?;
        for _ in 0..4 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":10.0,"wheel_unit":"line"}),
            )?;
        }
        let focused = snapshot(&h, id(&session))?;
        ensure!(
            focused["focused"] == true
                && focused["offset"].as_u64().unwrap_or(0) > 0
                && focused["samples"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|n| n.as_u64().is_some_and(|n| n < 120))),
            "Focused Codex history is inaccessible: {focused}"
        );
        observations.push(json!({"phase":"focused-up","state":focused}));
        for _ in 0..6 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":-10.0,"wheel_unit":"line"}),
            )?;
        }
        let focused_latest = snapshot(&h, id(&session))?;
        ensure!(
            focused_latest["focused"] == true
                && focused_latest["samples"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(160))),
            "Focused scrolling did not return to the latest output: {focused_latest}"
        );
        observations.push(json!({"phase":"focused-latest","state":focused_latest}));
        action(
            &h,
            &mut actions,
            json!({"at_ms":0,"target":format!("terminal:{}",id(&idle)),"hover":true}),
        )?;

        action(
            &h,
            &mut actions,
            json!({"at_ms":0,"target":format!("terminal:{}",id(&idle))}),
        )?;
        for _ in 0..8 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":10.0,"wheel_unit":"line"}),
            )?;
            observations.push(json!({"phase":"hover-up","state":snapshot(&h,id(&session))?}));
        }
        let earlier = snapshot(&h, id(&session))?;
        fs::write(&evidence_path, serde_json::to_vec_pretty(&observations)?)?;
        ensure!(
            earlier["focused"] == false,
            "Hover scrolling changed typing focus"
        );
        ensure!(
            earlier["samples"]
                .as_array()
                .is_some_and(|v| v.iter().any(|n| n.as_u64().is_some_and(|n| n < 120))),
            "Earlier live Codex messages were not reached in {mode}: {earlier}"
        );
        for _ in 0..12 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":-10.0,"wheel_unit":"line"}),
            )?;
        }
        let recent = snapshot(&h, id(&session))?;
        ensure!(
            recent["samples"]
                .as_array()
                .is_some_and(|v| v.contains(&json!(160))),
            "Downward scrolling did not reach recent output: {recent}"
        );
        observations.push(json!({"phase":"back-to-latest","state":recent}));
        fs::write(&evidence_path, serde_json::to_vec_pretty(&observations)?)?;
        if focus_only {
            ui_control::rpc(
                &Paths::at(h.root.clone()),
                ui_control::Request::Window {
                    action: "close".into(),
                },
            )?;
            wait_child(&mut native.0, Duration::from_secs(8))?;
            h.assert_pids(&[idle, session])?;
            println!("PASS fresh Codex {mode}: focused and hovered history navigation");
            continue;
        }
        let second = "Output a fenced text block with exactly 100 more lines. Each starts with TUPDATE immediately followed by its three-digit line number (1 through 100), then a space and public update. Do not use tools or write files. No extra commentary.";
        input = attach(&h, &session)?;
        h.write(&mut input, &format!("\x1b[200~{second}\x1b[201~"))?;
        thread::sleep(Duration::from_millis(500));
        h.write(&mut input, "\r")?;
        println!("Live Codex {mode}: second prompt submitted separately from paste");
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        loop {
            let text = h.history(id(&session))?;
            if text.contains("TUPDATE001") || text.contains("TUPDATE100") {
                break;
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "Second sample did not start"
            );
            thread::sleep(Duration::from_millis(100));
        }
        println!("Live Codex {mode}: second response started");
        for _ in 0..5 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":10.0,"wheel_unit":"line"}),
            )?;
        }
        let during = snapshot(&h, id(&session))?;
        let streamed = markers(&h.history(id(&session))?, "TUPDATE", 100);
        ensure!(
            !streamed.is_empty() && !streamed.contains(&100),
            "Response completed before a live streaming observation"
        );
        ensure!(
            during["samples"].as_array().is_some_and(|v| !v.is_empty()),
            "Earlier conversation was not visible during new output"
        );
        observations
            .push(json!({"phase":"during-output","state":during,"daemon_update_markers":streamed}));
        thread::sleep(Duration::from_secs(4));
        let after = snapshot(&h, id(&session))?;
        ensure!(
            after["samples"] == during["samples"],
            "New Codex output moved the retained-history view"
        );
        observations.push(json!({"phase":"after-output-wait","state":after}));
        for _ in 0..20 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":-10.0,"wheel_unit":"line"}),
            )?;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !snapshot(&h, id(&session))?["updates"]
            .as_array()
            .is_some_and(|v| v.contains(&json!(100)))
        {
            ensure!(
                std::time::Instant::now() < deadline,
                "Latest update was not reachable"
            );
            thread::sleep(Duration::from_millis(250));
        }
        let latest_update = snapshot(&h, id(&session))?;
        observations.push(json!({"phase":"latest-update","state":latest_update}));
        ui_control::rpc(
            &Paths::at(h.root.clone()),
            ui_control::Request::Window {
                action: "close".into(),
            },
        )?;
        wait_child(&mut native.0, Duration::from_secs(8))?;
        fs::write(h.root.join("actions.json"), "[]")?;
        let mut native = gui(&h, &o.output, &format!("{mode}-reconnect"))?;
        h.wait(
            |_| snapshot(&h, id(&session)).is_ok_and(|s| s["rect"].is_array()),
            15,
        )?;
        thread::sleep(Duration::from_secs(1));
        let reconnected = snapshot(&h, id(&session))?;
        // alacritty TermMode: cursor/application mouse, bracketed paste, focus,
        // alternate screen and alternate scroll routing must survive attachment.
        let routing_modes = (1_u64 << 1)
            | (1 << 3)
            | (1 << 4)
            | (1 << 5)
            | (1 << 6)
            | (1 << 11)
            | (1 << 12)
            | (1 << 13)
            | (1 << 15);
        ensure!(
            latest_update["modes"].as_u64().unwrap() & routing_modes
                == reconnected["modes"].as_u64().unwrap() & routing_modes,
            "Terminal input modes changed after reconnect: {latest_update} -> {reconnected}"
        );
        observations.push(json!({"phase":"reconnected","state":reconnected}));
        let mut actions = Vec::new();
        action(
            &h,
            &mut actions,
            json!({"at_ms":0,"target":format!("terminal:{}",id(&idle))}),
        )?;
        for _ in 0..12 {
            action(
                &h,
                &mut actions,
                json!({"at_ms":0,"target":target,"scroll":10.0,"wheel_unit":"line"}),
            )?;
        }
        observations
            .push(json!({"phase":"reconnected-hover-up","state":snapshot(&h,id(&session))?}));
        fs::write(&evidence_path, serde_json::to_vec_pretty(&observations)?)?;
        ui_control::rpc(
            &Paths::at(h.root.clone()),
            ui_control::Request::Window {
                action: "close".into(),
            },
        )?;
        wait_child(&mut native.0, Duration::from_secs(8))?;
        h.assert_pids(&[idle, session])?;
        println!(
            "PASS live Codex {mode}: hover history, focus isolation, new output and GUI reconnect"
        );
    }
    Ok(())
}
