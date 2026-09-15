//! Real PTYs: generated prompt hooks, busy builtins, queued input and all-target preflight.
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
            let mut debug = h.attach(&initial)?;
            h.write(
                &mut debug,
                "declare -p PROMPT_COMMAND _terminator_epoch; trap -p DEBUG; jobs -l; printf 'subjobs=<%s>\\n' \"$(jobs -p)\"\r",
            )?;
            thread::sleep(Duration::from_millis(300));
            anyhow::bail!(
                "Initial {name} prompt did not close: {outcome}; isolated diagnostics: {}",
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
            "{name} prompt after a completed builtin did not close: {outcome}"
        );
        let busy = h.shell(&project)?;
        let idle = h.shell(&project)?;
        thread::sleep(Duration::from_millis(400));
        let mut stream = h.attach(&busy)?;
        h.write(&mut stream, "read value\r")?;
        thread::sleep(Duration::from_millis(300));
        let outcome = close(&h, &[&idle, &busy], &generation)?;
        ensure!(!closed(&outcome), "Builtin mistaken for idle: {outcome}");
        h.assert_pids(&[idle.clone(), busy.clone()])?;
        let stale = close(&h, &[&idle], "stale-generation")?;
        ensure!(
            stale[0]["status"] == "stale_generation",
            "Stale generation was accepted"
        );
        h.assert_pids(std::slice::from_ref(&idle))?;
        h.write(&mut stream, "done\r")?;
        thread::sleep(Duration::from_millis(400));
        // Input used by a builtin is deliberately not guessed as a shell command boundary.
        // A conservative fallback is acceptable here, but never a close while read was waiting.
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
        let partial = h.shell(&project)?;
        thread::sleep(Duration::from_millis(300));
        h.write(&mut h.attach(&partial)?, "echo")?;
        thread::sleep(Duration::from_millis(100));
        ensure!(
            !closed(&close(&h, &[&partial], &generation)?),
            "Partially entered input was discarded"
        );
        h.assert_pids(std::slice::from_ref(&partial))?;
        h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"idle-fixture-agent","terminal_session_id":id(&idle),"agent_invocation_id":"idle-fixture-agent","agent_kind":"sample","state":"waiting_input","request_id":"question","sequence":1,"summary":"Fixture agent awaits input","details":"Isolated test","resume":null}}))?;
        ensure!(
            !closed(&close(&h, &[&idle], &generation)?),
            "Active agent ignored"
        );
        h.assert_pids(std::slice::from_ref(&idle))?;
        println!(
            "PASS idle-close {name}: initial and completed prompts, builtin, tab preflight, foreground/background/stopped jobs, partial input, agent, stale generation"
        );
    }
    Ok(())
}
