//! Real PTYs: idle shells, children, all-target preflight, and agents.
use crate::harness::{Harness, id, session};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, thread, time::Duration};

fn close(h: &Harness, targets: &[&Value], generation: &str) -> Result<Value> {
    h.rpc(json!({"CloseIdleSessions":{"generation":generation,"sessions":targets.iter().map(|s| id(s)).collect::<Vec<_>>()}}))?
        .get("IdleSessionsClosed")
        .cloned()
        .ok_or_else(|| anyhow!("missing IdleSessionsClosed"))
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
    thread::scope(|scope| -> Result<()> {
        let mut tasks = Vec::new();
        for name in ["zsh", "bash", "fish", "sh"] {
            tasks.push((name, scope.spawn(move || run_shell(name))));
        }
        tasks.push(("zsh-prompt-framework", scope.spawn(zsh_prompt_framework)));
        let mut failures = Vec::new();
        for (name, task) in tasks {
            match task.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{name}: {error:#}")),
                Err(_) => failures.push(format!("{name}: panicked")),
            }
        }
        ensure!(
            failures.is_empty(),
            "idle-close failures:\n{}",
            failures.join("\n")
        );
        Ok(())
    })
}

/// One shell kind's isolated daemon, project and current generation.
struct Idle<'a> {
    name: &'a str,
    h: Harness,
    project: Value,
    generation: String,
}

fn run_shell(name: &str) -> Result<()> {
    let Some(shell) = terminator_core::find_executable(name) else {
        println!("SKIP idle-close {name}: not installed");
        return Ok(());
    };
    let t = Idle::launch(name, &shell)?;
    let initial = t.h.shell(&t.project)?;
    thread::sleep(Duration::from_millis(700));
    let outcome = t.close(&[&initial])?;
    if name == "sh" {
        ensure!(
            !closed(&outcome),
            "Unsupported shell was closed without readiness"
        );
        t.h.assert_pids(std::slice::from_ref(&initial))?;
        println!("PASS idle-close sh: unsupported shell retained");
        return Ok(());
    }
    if !closed(&outcome) {
        anyhow::bail!(
            "Initial {name} shell did not close: {outcome}; {}",
            t.h.history(id(&initial))?
        );
    }
    t.builtins()?;
    let idle = t.busy_sibling_and_stale()?;
    t.background_jobs()?;
    t.foreground_job()?;
    t.submitted_foreground_job()?;
    t.partial_input()?;
    t.active_agent(&idle)?;
    println!(
        "PASS idle-close {name}: idle shells, builtin without children, tab preflight, foreground/background/stopped jobs, submitted foreground job, partial input, agent, stale generation"
    );
    Ok(())
}

impl<'a> Idle<'a> {
    fn launch(name: &'a str, shell: &std::path::Path) -> Result<Self> {
        let mut h = Harness::new()?;
        let home = h.root.join("home");
        fs::create_dir_all(&home)?;
        if name == "fish" {
            // A fresh fish profile otherwise starts a background Python job
            // to generate completions. Test an initialized, idle profile.
            fs::create_dir_all(h.root.join("data/fish/generated_completions"))?;
        }
        h.env
            .insert("HOME".into(), home.to_string_lossy().into_owned());
        h.env
            .insert("ZDOTDIR".into(), home.to_string_lossy().into_owned());
        h.restart()?;
        h.set_setting("shell", json!(shell))?;
        let project = h.project(&format!("idle-{name}"))?;
        let generation = h
            .state()?
            .get("generation")
            .ok_or_else(|| anyhow!("missing generation"))?
            .as_str()
            .unwrap()
            .to_owned();
        Ok(Self {
            name,
            h,
            project,
            generation,
        })
    }
    fn close(&self, targets: &[&Value]) -> Result<Value> {
        close(&self.h, targets, &self.generation)
    }
    /// A completed builtin and a waiting `read` both leave the shell idle.
    fn builtins(&self) -> Result<()> {
        let (h, name) = (&self.h, self.name);
        let completed = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(300));
        let mut command = h.attach(&completed)?;
        h.write(&mut command, "true\r")?;
        thread::sleep(Duration::from_millis(400));
        let outcome = self.close(&[&completed])?;
        ensure!(
            closed(&outcome),
            "{name} shell after a completed builtin did not close: {outcome}"
        );
        let builtin = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(400));
        h.write(&mut h.attach(&builtin)?, "read value\r")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            closed(&self.close(&[&builtin])?),
            "Builtin without children required confirmation"
        );
        Ok(())
    }
    /// A busy sibling blocks the whole close, and a stale generation is refused;
    /// returns the idle sibling.
    fn busy_sibling_and_stale(&self) -> Result<Value> {
        let h = &self.h;
        let busy = h.shell(&self.project)?;
        let idle = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(400));
        h.write(&mut h.attach(&busy)?, "sleep 30\r")?;
        thread::sleep(Duration::from_millis(300));
        let outcome = self.close(&[&idle, &busy])?;
        ensure!(
            !closed(&outcome),
            "Busy sibling did not block idle close: {outcome}"
        );
        h.assert_pids(&[idle.clone(), busy.clone()])?;
        let stale = close(h, &[&idle], "stale-generation")?;
        ensure!(
            stale
                .get(0)
                .and_then(|outcome| outcome.get("status"))
                .is_some_and(|status| status == "stale_generation"),
            "Stale generation was accepted"
        );
        h.assert_pids(std::slice::from_ref(&idle))?;
        Ok(idle)
    }
    fn background_jobs(&self) -> Result<()> {
        let h = &self.h;
        let background = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(300));
        let mut bg = h.attach(&background)?;
        h.write(&mut bg, "sleep 30 &\r")?;
        thread::sleep(Duration::from_millis(300));
        let outcome = self.close(&[&background])?;
        ensure!(!closed(&outcome), "Background job mistaken for idle");
        h.assert_pids(std::slice::from_ref(&background))?;
        let last_pid = if self.name == "fish" {
            "$last_pid"
        } else {
            "$!"
        };
        h.write(
            &mut bg,
            &format!("kill -STOP {last_pid} && printf '%s\\n' 'IDLE_STOPPED_''JOB'\r"),
        )?;
        h.wait(
            |_| {
                h.history(id(&background))
                    .is_ok_and(|s| s.contains("IDLE_STOPPED_JOB"))
            },
            5,
        )?;
        ensure!(
            !closed(&self.close(&[&background])?),
            "Stopped job mistaken for idle"
        );
        Ok(())
    }
    fn foreground_job(&self) -> Result<()> {
        let h = &self.h;
        let foreground = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(300));
        h.write(&mut h.attach(&foreground)?, "sleep 30\r")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            !closed(&self.close(&[&foreground])?),
            "Foreground job mistaken for idle"
        );
        h.assert_pids(std::slice::from_ref(&foreground))?;
        Ok(())
    }
    // A forwarded command is only observable once its job owns the terminal.
    // Shells can buffer forwarded input briefly (fish reads it on a worker
    // thread), and the daemon cannot see buffered input. Wait for the
    // foreground job's output instead of closing inside that window; the
    // echoed command line would repeat a literal marker, so the marker is
    // assembled from arguments to prove it came from the job itself.
    fn submitted_foreground_job(&self) -> Result<()> {
        let h = &self.h;
        let submitted = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(300));
        let mut input = h.attach(&submitted)?;
        h.write(
            &mut input,
            "sh -c 'printf \"IDLE_%s\\n\" FOREGROUND_JOB; sleep 30'\r",
        )?;
        terminator_core::write_frame(&mut input, &json!({"Resize":{"rows":31,"cols":91}}))?;
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(3))
            .ok_or_else(|| anyhow!("input deadline overflow"))?;
        while session(&h.state()?, id(&submitted))["rows"] != 31 {
            ensure!(
                std::time::Instant::now() < deadline,
                "Input forwarding was not acknowledged"
            );
            thread::yield_now();
        }
        h.wait(
            |_| {
                h.history(id(&submitted))
                    .is_ok_and(|s| s.contains("IDLE_FOREGROUND_JOB"))
            },
            5,
        )?;
        ensure!(
            !closed(&self.close(&[&submitted])?),
            "Submitted foreground job was closed"
        );
        h.assert_pids(std::slice::from_ref(&submitted))?;
        Ok(())
    }
    fn partial_input(&self) -> Result<()> {
        let h = &self.h;
        let partial = h.shell(&self.project)?;
        thread::sleep(Duration::from_millis(300));
        h.write(&mut h.attach(&partial)?, "echo")?;
        thread::sleep(Duration::from_millis(100));
        ensure!(
            closed(&self.close(&[&partial])?),
            "Partial input without children required confirmation"
        );
        Ok(())
    }
    fn active_agent(&self, idle: &Value) -> Result<()> {
        let h = &self.h;
        h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"idle-fixture-agent","terminal_session_id":id(idle),"agent_invocation_id":"idle-fixture-agent","agent_kind":"sample","state":"waiting_input","request_id":"question","sequence":1,"summary":"Fixture agent awaits input","details":"Isolated test","resume":null}}))?;
        ensure!(!closed(&self.close(&[idle])?), "Active agent ignored");
        h.assert_pids(std::slice::from_ref(idle))?;
        Ok(())
    }
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
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert("shell".into(), json!(shell));
    h.rpc(json!({"Settings": settings}))?;
    let project = h.project("idle-zsh-framework")?;
    let generation = h
        .state()?
        .get("generation")
        .ok_or_else(|| anyhow!("missing generation"))?
        .as_str()
        .unwrap()
        .to_owned();
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
