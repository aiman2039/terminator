//! Review regressions use fresh daemons, PTYs and temporary files only.
use crate::harness::*;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Read, os::unix::fs::PermissionsExt, thread, time::Duration};
use terminator_core::{quote, read_frame};

pub fn run() -> Result<()> {
    idle_shutdown_race()?;
    history_burst()?;
    missing_helper_health()?;
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

fn history_burst() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("history-burst")?;
    let shell = h.shell(&project)?;
    let mut stream = h.attach(&shell)?;
    h.write(&mut stream, "stty -echo; printf 'BURST_READY\\n'\n")?;
    h.wait(
        |_| {
            h.history(id(&shell))
                .is_ok_and(|s| s.contains("BURST_READY"))
        },
        5,
    )?;
    // Six MiB exceeds the entire queue capacity. Octal avoids echoing any payload Xs.
    h.write(
        &mut stream,
        "head -c 6291456 /dev/zero | tr '\\000' '\\130'; printf '\\nHISTORY_BURST_DONE\\n'\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&shell))
                .is_ok_and(|s| s.contains("HISTORY_BURST_DONE"))
        },
        30,
    )?;
    h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
    let state = h.wait(|s| session(s, id(&shell))["lifecycle"] == "ended", 5)?;
    ensure!(
        session(&state, id(&shell))["truncated"] == false,
        "History burst was truncated"
    );
    h.history(id(&shell))?; // Ended History requests flush the storage worker.
    let mut saved = 0;
    for entry in fs::read_dir(terminator_core::Paths::at(h.root.clone()).history_dir())? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(id(&shell)) {
            saved += fs::read(entry.path())?
                .iter()
                .filter(|b| **b == b'X')
                .count();
        }
    }
    ensure!(
        saved == 6_291_456,
        "Expected every burst byte in saved history, got {saved}"
    );
    println!(
        "{}",
        json!({"history_burst_bytes_saved":saved,"truncated":false})
    );
    Ok(())
}

fn missing_helper_health() -> Result<()> {
    let mut h = Harness::new()?;
    let install = h.root.join("isolated-install");
    fs::create_dir(&install)?;
    for name in ["terminator-daemon", "terminator-hook"] {
        fs::copy(bin().join(name), install.join(name))?;
    }
    h.env.insert(
        "TERMINATOR_FIXTURE_DAEMON".into(),
        install
            .join("terminator-daemon")
            .to_string_lossy()
            .into_owned(),
    );
    h.restart()?;
    h.setup()?;
    let project = h.project("helper-health")?;
    let shell = h.shell(&project)?;
    let initial = h.state()?;
    ensure!(
        initial["attachment_helper_available"] == true,
        "Installed helper was not available"
    );
    let pinned =
        std::path::PathBuf::from(initial["attachment_helper_executable"].as_str().unwrap());
    ensure!(
        pinned.starts_with(&h.root) && !pinned.starts_with(&install),
        "Helper was not pinned outside the installation"
    );
    let original_helper = fs::read(&pinned)?;
    // Model both in-place replacement and removal of an entire old app bundle.
    fs::write(
        install.join("terminator-hook"),
        b"replaced application helper",
    )?;
    fs::set_permissions(
        install.join("terminator-hook"),
        fs::Permissions::from_mode(0o600),
    )?;
    fs::remove_dir_all(&install)?;
    ensure!(
        h.state()?["attachment_helper_available"] == true && fs::read(&pinned)? == original_helper,
        "App replacement or removal changed the pinned helper"
    );
    let second = h.shell(&project)?;
    let new_cwd = h.root.join("after-app-removal");
    fs::create_dir(&new_cwd)?;
    let mut stream = h.attach(&shell)?;
    h.write(
        &mut stream,
        &format!(
            "cd {}; {} cwd \"$PWD\"; printf 'PINNED_HELPER_STILL_WORKS\\n'\n",
            quote(&new_cwd.to_string_lossy()),
            quote(&pinned.to_string_lossy())
        ),
    )?;
    h.wait(
        |st| session(st, id(&shell))["cwd"] == new_cwd.to_string_lossy().as_ref(),
        5,
    )?;
    h.assert_pids(&[shell.clone(), second.clone()])?;
    ensure!(
        h.state()?["generation"] == initial["generation"],
        "App removal replaced the daemon"
    );
    // Health now refers to the actual private helper, not a discarded bundle.
    let hint = serde_json::from_value::<terminator_core::State>(h.state()?)?.snapshot_hint();
    fs::set_permissions(&pinned, fs::Permissions::from_mode(0o600))?;
    let paths = terminator_core::Paths::at(h.root.clone());
    let response = terminator_core::conditional_snapshot(&paths, Some(hint))?;
    ensure!(
        matches!(response, terminator_core::Response::State(ref s) if s.attachment_helper_available == Some(false)),
        "Permission change did not invalidate conditional snapshot"
    );
    fs::remove_file(&pinned)?;
    ensure!(
        h.state()?["attachment_helper_available"] == false,
        "Removed helper was not detected"
    );
    h.assert_pids(std::slice::from_ref(&shell))?;
    ensure!(
        h.rpc(json!("ShutdownIfIdle")).is_err(),
        "Broken helper allowed shutdown with a live session"
    );
    ensure!(
        h.shell(&project).is_err(),
        "Missing helper should give an installation error"
    );
    h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
    h.rpc(json!({"Stop":{"session":id(&second)}}))?;
    h.wait(
        |s| {
            session(s, id(&shell))["lifecycle"] == "ended"
                && session(s, id(&second))["lifecycle"] == "ended"
        },
        5,
    )?;
    h.rpc(json!("ShutdownIfIdle"))?;
    println!(
        "{}",
        json!({"app_removal_preserves_helper":true,"new_session_after_removal":true,"pinned_hook_executes":true,"missing_helper_detected":true,"permission_change_invalidates_snapshot":true,"live_sessions_preserved":true})
    );
    Ok(())
}
