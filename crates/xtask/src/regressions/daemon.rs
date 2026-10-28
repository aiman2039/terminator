use crate::harness::{Harness, id, session, session_present};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, io::Read, thread, time::Duration};
use terminator_core::{quote, read_frame};

pub(super) fn large_snapshots() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("notifications")?;
    let shell = h.shell(&project)?;
    let details = "x".repeat(65536);
    for n in 0..130 {
        h.rpc(json!({"Hook":{
            "protocol_version":1,"event_id":format!("event-{n}"),
            "terminal_session_id":id(&shell),"agent_invocation_id":format!("fixture-{n}"),
            "agent_kind":"custom","provider_session_id":"fixture",
            "state":"waiting_input","request_id":format!("request-{n}"),
            "sequence":n,"summary":"Pending fixture request","details":details,
            "resume":{"program":"codex","args":["resume","fixture"]}
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
        response
            .get("Error")
            .and_then(Value::as_str)
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

pub(super) fn stalled_attachment() -> Result<()> {
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
    let mut total = 0usize;
    loop {
        let n = stream.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n);
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

pub(super) fn terminal_editors() -> Result<()> {
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
        #[cfg(unix)]
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700))?;
        let mut settings = h
            .state()?
            .get("settings")
            .cloned()
            .ok_or_else(|| anyhow!("missing settings"))?;
        let settings_object = settings
            .as_object_mut()
            .ok_or_else(|| anyhow!("settings is not an object"))?;
        settings_object.insert("editor_mode".into(), json!("Terminal"));
        settings_object.insert("editor_program".into(), json!(program));
        h.rpc(json!({"Settings":settings}))?;
        let editor = h
            .rpc(json!({"Create":{
                "project":id(&project),"cwd":null,"file":file,"line":12,"column":3,"editor":true
            }}))?
            .get("Created")
            .cloned()
            .ok_or_else(|| anyhow!("missing Created"))?;
        h.wait(|st| !session_present(st, id(&editor)), 5)?;
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

pub(super) fn history_burst() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("history-burst")?;
    let shell = h.shell(&project)?;
    let mut stream = h.attach(&shell)?;
    // Markers are assembled at runtime (`printf '%s' ...`) so the echoed command
    // line never contains the literal token. Otherwise the readiness wait is
    // satisfied by the echo before `stty -echo` or the pipeline has run.
    h.write(&mut stream, "stty -echo; printf 'BURST%s\\n' _READY\n")?;
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
        "head -c 6291456 /dev/zero | tr '\\000' '\\130'; printf '\\nHISTORY_%s\\n' BURST_DONE\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&shell))
                .is_ok_and(|s| s.contains("HISTORY_BURST_DONE"))
        },
        30,
    )?;
    // Give the shell an agent resume handle so it is not pruned with its
    // scrollback: this fixture measures storage capacity, not retention policy.
    h.rpc(json!({"Hook":{
        "protocol_version":1,"event_id":"burst-resume",
        "terminal_session_id":id(&shell),"agent_invocation_id":"burst-agent",
        "agent_kind":"custom","provider_session_id":"burst",
        "state":"waiting_input","request_id":"burst","sequence":1,
        "summary":"Burst fixture","details":"",
        "resume":{"program":"codex","args":["resume","burst"]}
    }}))?;
    h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
    let state = h.wait(|s| session(s, id(&shell))["lifecycle"] == "ended", 5)?;
    ensure!(
        session(&state, id(&shell))["truncated"] == false,
        "History burst was truncated"
    );
    h.history(id(&shell))?; // Ended History requests flush the storage worker.
    let mut saved = 0usize;
    for entry in fs::read_dir(terminator_core::Paths::at(h.root.clone()).history_dir())? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(id(&shell)) {
            saved = saved.saturating_add(
                fs::read(entry.path())?
                    .iter()
                    .filter(|b| **b == b'X')
                    .count(),
            );
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
