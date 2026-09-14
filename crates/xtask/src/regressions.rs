//! Review regressions use fresh daemons, PTYs and temporary files only.
use crate::harness::*;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Read, os::unix::fs::PermissionsExt, thread, time::Duration};
use terminator_core::{quote, read_frame};

pub fn run() -> Result<()> {
    idle_shutdown_race()?;
    large_snapshots()?;
    stalled_attachment()?;
    terminal_editors()?;
    Ok(())
}

fn large_snapshots() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("notifications")?;
    let shell = h.shell(&project)?;
    let details = "x".repeat(65536);
    for n in 0..130 {
        h.rpc(json!({"Hook":{
            "protocol_version":1,"event_id":format!("event-{n}"),
            "terminal_session_id":id(&shell),"agent_invocation_id":"fixture",
            "agent_kind":"custom","provider_session_id":"fixture",
            "state":"waiting_input","request_id":format!("request-{n}"),
            "sequence":n,"summary":"Pending fixture request","details":details,"resume":null
        }}))?;
    }
    let check = |state: &Value| -> Result<()> {
        let notices = state["notifications"].as_array().unwrap();
        ensure!(notices.len() == 130, "Pending notifications were lost");
        ensure!(
            notices
                .iter()
                .all(|n| n["details"] == details && n["dismissed"] == false),
            "Large snapshot changed notification details or dismissal"
        );
        Ok(())
    };
    let state = h.state()?;
    ensure!(
        serde_json::to_vec(&state)?.len() > terminator_core::MAX_FRAME,
        "Fixture must exceed one frame"
    );
    check(&state)?;
    let response: Value = read_frame(&mut h.connect(json!("Snapshot"), None, None)?)?;
    ensure!(
        response["Error"]
            .as_str()
            .is_some_and(|e| e.contains("update")),
        "Legacy client did not receive an actionable size error"
    );
    let paths = terminator_core::Paths::at(h.root.clone());
    let hint = serde_json::from_value::<terminator_core::State>(state)?.snapshot_hint();
    ensure!(
        matches!(
            terminator_core::conditional_snapshot(&paths, Some(hint))?,
            terminator_core::Response::Unchanged
        ),
        "Conditional snapshot changed"
    );
    h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
    h.wait(|st| session(st, id(&shell))["lifecycle"] == "ended", 5)?;
    h.restart()?;
    check(&h.state()?)?;
    println!(
        "{}",
        json!({"large_snapshot":true,"pending_details_preserved_after_restart":true,"legacy_size_error":true})
    );
    Ok(())
}

fn stalled_attachment() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("stalled-output")?;
    let shell = h.shell(&project)?;
    let mut stream = h.attach(&shell)?;
    // Darwin rejects changing socket timeouts after the peer has shut down.
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    h.write(
        &mut stream,
        "stty -echo; head -c 2000000 /dev/zero | tr '\\0' x; printf '\\nBURST_DONE\\n'\n",
    )?;
    // Exceed the daemon writer deadline without reading any output.
    thread::sleep(Duration::from_secs(4));
    let mut bytes = [0; 65536];
    let mut total = 0;
    loop {
        let n = stream.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        total += n;
        ensure!(total < 4 * 1024 * 1024, "Stalled output did not disconnect");
    }
    ensure!(
        h.write(&mut stream, "printf 'SHOULD_NOT_EXECUTE\\n'\n")
            .is_err(),
        "Input remained connected after output failed"
    );
    let mut reattached = h.attach(&shell)?;
    h.write(&mut reattached, "printf '\\nREATTACHED_AFTER_TIMEOUT\\n'\n")?;
    h.wait(
        |_| {
            h.history(id(&shell))
                .is_ok_and(|s| s.contains("REATTACHED_AFTER_TIMEOUT"))
        },
        5,
    )?;
    h.assert_pids(&[shell])?;
    println!(
        "{}",
        json!({"stalled_attachment_closes_both_directions":true,"reattach_same_pid":true})
    );
    Ok(())
}

fn terminal_editors() -> Result<()> {
    let h = Harness::new()?;
    let project = h.project("editors")?;
    let file = h.root.join("editors/- literal file.txt");
    fs::write(&file, "fixture\n")?;
    for name in ["nano", "pico", "custom-editor"] {
        let program = h.root.join(name);
        let capture = h.root.join(format!("{name}-args"));
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n",
                quote(&capture.to_string_lossy())
            ),
        )?;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700))?;
        let mut settings = h.state()?["settings"].clone();
        settings["editor_mode"] = json!("Terminal");
        settings["editor_program"] = json!(program);
        h.rpc(json!({"Settings":settings}))?;
        let editor = h.rpc(json!({"Create":{
            "project":id(&project),"cwd":null,"file":file,"line":12,"column":3,"editor":true
        }}))?["Created"]
            .clone();
        h.wait(|st| session(st, id(&editor))["lifecycle"] == "ended", 5)?;
        let args = fs::read_to_string(&capture)?;
        let expected = format!(
            "{}{}\n",
            if name == "custom-editor" { "" } else { "+12\n" },
            file.display()
        );
        ensure!(args == expected, "Incorrect arguments for {name}: {args:?}");
    }
    println!(
        "{}",
        json!({"terminal_editor_literal_file_and_position":true})
    );
    Ok(())
}

fn idle_shutdown_race() -> Result<()> {
    use std::sync::{Arc, Barrier};
    use terminator_core::{Paths, Request, Response, rpc};
    for _ in 0..8 {
        let h = Harness::new()?;
        h.setup()?;
        let project = h.project("idle-shutdown-race")?;
        let barrier = Arc::new(Barrier::new(2));
        let first = barrier.clone();
        let paths = Paths::at(h.root.clone());
        let project_id = id(&project).to_owned();
        let creator = thread::spawn(move || {
            first.wait();
            rpc(
                &paths,
                Request::Create {
                    project: project_id,
                    cwd: None,
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
            )
        });
        barrier.wait();
        let shutdown = rpc(&Paths::at(h.root.clone()), Request::ShutdownIfIdle);
        let creation = creator.join().unwrap();
        match creation {
            Ok(Response::Created(session)) => {
                ensure!(
                    shutdown.is_err(),
                    "Idle shutdown acknowledged despite a successful creation"
                );
                let state = h.state()?;
                ensure!(
                    state["generation"] == session.generation,
                    "Daemon changed under a live session"
                );
                h.rpc(json!({"Stop":{"session":session.id}}))?;
            }
            Err(_) => ensure!(
                matches!(shutdown, Ok(Response::Ok)),
                "Neither concurrent operation succeeded"
            ),
            _ => anyhow::bail!("Unexpected creation reply"),
        }
    }
    println!("Idle shutdown serialized against concurrent session creation (8 runs)");
    Ok(())
}
