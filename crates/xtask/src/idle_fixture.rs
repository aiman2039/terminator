//! Real PTYs: idle shells, children, all-target preflight, and agents.
use crate::harness::*;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, thread, time::Duration};

fn close(h: &Harness, targets: &[&Value], generation: &str) -> Result<Value> {
    Ok(h.rpc(json!({"CloseIdleSessions":{"generation":generation,"sessions":targets.iter().map(|s| id(s)).collect::<Vec<_>>()}}))?["IdleSessionsClosed"].clone())
}
fn closed(value: &Value) -> bool {
    value.as_array().is_some_and(|outcomes| {
        !outcomes.is_empty()
            && outcomes
                .iter()
                .all(|o| matches!(o["status"].as_str(), Some("closed" | "already_ended")))
    })
}
pub fn run() -> Result<()> {
    for name in ["zsh", "bash", "fish", "sh"] {
        let Some(shell) = terminator_core::find_executable(name) else {
            println!("SKIP idle-close {name}: not installed");
            continue;
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
        let project = h.project(&format!("idle-{name}"))?;
        let generation = h.state()?["generation"].as_str().unwrap().to_owned();
        let initial = h.shell(&project)?;
        thread::sleep(Duration::from_millis(700));
        let outcome = close(&h, &[&initial], &generation)?;
        if name == "sh" {
            ensure!(
                !closed(&outcome),
                "Unsupported shell was closed without readiness"
            );
            h.assert_pids(std::slice::from_ref(&initial))?;
            println!("PASS idle-close sh: unsupported shell retained");
            continue;
        }
        if !closed(&outcome) {
            anyhow::bail!(
                "Initial {name} shell did not close: {outcome}; {}",
                h.history(id(&initial))?
            );
        }
        let completed = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        let mut command = h.attach(&completed)?;
        h.write(&mut command, "true\r")?;
        thread::sleep(Duration::from_millis(400));
        let outcome = close(&h, &[&completed], &generation)?;
        ensure!(
            closed(&outcome),
            "{name} shell after a completed builtin did not close: {outcome}"
        );
        let builtin = h.shell(&project)?;
        thread::sleep(Duration::from_millis(400));
        h.write(&mut h.attach(&builtin)?, "read value\r")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            closed(&close(&h, &[&builtin], &generation)?),
            "Builtin without children required confirmation"
        );
        let busy = h.shell(&project)?;
        let idle = h.shell(&project)?;
        thread::sleep(Duration::from_millis(400));
        h.write(&mut h.attach(&busy)?, "sleep 30\r")?;
        thread::sleep(Duration::from_millis(300));
        let outcome = close(&h, &[&idle, &busy], &generation)?;
        ensure!(
            !closed(&outcome),
            "Busy sibling did not block idle close: {outcome}"
        );
        h.assert_pids(&[idle.clone(), busy.clone()])?;
        let stale = close(&h, &[&idle], "stale-generation")?;
        ensure!(
            stale[0]["status"] == "stale_generation",
            "Stale generation was accepted"
        );
        h.assert_pids(std::slice::from_ref(&idle))?;
        let background = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        let mut bg = h.attach(&background)?;
        h.write(&mut bg, "sleep 30 &\r")?;
        thread::sleep(Duration::from_millis(300));
        let outcome = close(&h, &[&background], &generation)?;
        ensure!(!closed(&outcome), "Background job mistaken for idle");
        h.assert_pids(std::slice::from_ref(&background))?;
        h.write(&mut bg, "kill -STOP $!\r")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            !closed(&close(&h, &[&background], &generation)?),
            "Stopped job mistaken for idle"
        );
        let foreground = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        h.write(&mut h.attach(&foreground)?, "sleep 30\r")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            !closed(&close(&h, &[&foreground], &generation)?),
            "Foreground job mistaken for idle"
        );
        h.assert_pids(std::slice::from_ref(&foreground))?;
        // Resize is processed on the same attachment after Input. Observing it
        // acknowledges forwarding, without waiting for the command to settle.
        let submitted = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        let mut input = h.attach(&submitted)?;
        h.write(&mut input, "sleep 30\r")?;
        terminator_core::write_frame(&mut input, &json!({"Resize":{"rows":31,"cols":91}}))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while session(&h.state()?, id(&submitted))["rows"] != 31 {
            ensure!(
                std::time::Instant::now() < deadline,
                "Input forwarding was not acknowledged"
            );
            thread::yield_now();
        }
        ensure!(
            !closed(&close(&h, &[&submitted], &generation)?),
            "Just-submitted command was closed"
        );
        h.assert_pids(std::slice::from_ref(&submitted))?;
        let partial = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        h.write(&mut h.attach(&partial)?, "echo")?;
        thread::sleep(Duration::from_millis(100));
        ensure!(
            closed(&close(&h, &[&partial], &generation)?),
            "Partial input without children required confirmation"
        );
        h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"idle-fixture-agent","terminal_session_id":id(&idle),"agent_invocation_id":"idle-fixture-agent","agent_kind":"sample","state":"waiting_input","request_id":"question","sequence":1,"summary":"Fixture agent awaits input","details":"Isolated test","resume":null}}))?;
        ensure!(
            !closed(&close(&h, &[&idle], &generation)?),
            "Active agent ignored"
        );
        h.assert_pids(std::slice::from_ref(&idle))?;
        println!(
            "PASS idle-close {name}: idle shells, builtin without children, tab preflight, foreground/background/stopped jobs, immediate close after input forwarding, partial input, agent, stale generation"
        );
    }
    zsh_prompt_framework()?;
    Ok(())
}

fn zsh_prompt_framework() -> Result<()> {
    let Some(shell) = terminator_core::find_executable("zsh") else {
        println!("SKIP idle-close zsh prompt framework: zsh not installed");
        return Ok(());
    };
    let mut h = Harness::new()?;
    let home = h.root.join("home");
    fs::create_dir_all(&home)?;
    let rc = if let Some(starship) = terminator_core::find_executable("starship") {
        format!("eval \"$('{}' init zsh)\"\n", starship.display())
    } else {
        "setopt promptsubst\nPROMPT='%# '\nzle-line-init() {}\nzle -N zle-line-init\n".into()
    };
    fs::write(home.join(".zshrc"), rc)?;
    h.env
        .insert("HOME".into(), home.to_string_lossy().into_owned());
    h.env
        .insert("ZDOTDIR".into(), home.to_string_lossy().into_owned());
    h.restart()?;
    let mut settings = h.state()?["settings"].clone();
    settings["shell"] = json!(shell);
    h.rpc(json!({"Settings": settings}))?;
    let project = h.project("idle-zsh-framework")?;
    let generation = h.state()?["generation"].as_str().unwrap().to_owned();
    let initial = h.shell(&project)?;
    thread::sleep(Duration::from_millis(1200));
    let outcome = close(&h, &[&initial], &generation)?;
    ensure!(
        closed(&outcome),
        "Idle zsh with prompt framework required confirmation: {outcome}; {}",
        h.history(id(&initial))?
    );
    let busy = h.shell(&project)?;
    thread::sleep(Duration::from_millis(800));
    h.write(&mut h.attach(&busy)?, "sleep 30\r")?;
    thread::sleep(Duration::from_millis(300));
    let outcome = close(&h, &[&busy], &generation)?;
    ensure!(
        !closed(&outcome),
        "Foreground child mistaken for idle under prompt framework: {outcome}"
    );
    h.assert_pids(std::slice::from_ref(&busy))?;
    println!("PASS idle-close zsh prompt framework: idle prompt closed, foreground child retained");
    Ok(())
}
