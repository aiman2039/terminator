use crate::harness::{Harness, artifacts, bin, git, id, output, session, sessions};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::{quote as shell_quote, read_frame};

pub fn run() -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let a = h.project("project-a")?;
    let b = h.project("project-b")?;
    let s = h.shell(&a)?;
    let mut stream = h.attach(&s)?;
    h.write(&mut stream, "printf 'RECONNECT_PROOF_123\\n'\n")?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|text| text.contains("RECONNECT_PROOF_123"))
        },
        5,
    )?;
    drop(stream);
    std::os::unix::fs::symlink(a["path"].as_str().unwrap(), h.root.join("alias-a"))?;
    for (path, expected) in [
        (PathBuf::from(b["path"].as_str().unwrap()), id(&b)),
        (h.root.join("alias-a"), id(&a)),
        (PathBuf::from(a["path"].as_str().unwrap()), id(&a)),
    ] {
        h.rpc(json!({"AddProject":{"path":path}}))?;
        let state = h.state()?;
        ensure!(
            state["selected_project"] == expected
                && state["projects"].as_array().unwrap().len() == 2,
            "Project selection must be idempotent: expected {expected}, selected {}, projects {}",
            state["selected_project"],
            state["projects"].as_array().unwrap().len()
        );
        h.assert_pids(std::slice::from_ref(&s))?;
    }
    for path in [h.root.join("missing"), h.root.join("daemon.log")] {
        ensure!(
            h.rpc(json!({"AddProject":{"path":path}})).is_err(),
            "Invalid project accepted"
        );
        ensure!(
            h.state()?["selected_project"] == id(&a),
            "Failed open changed selection"
        );
    }
    let mut stream = h.attach(&s)?;
    h.write(
        &mut stream,
        &format!(
            "cd {}; {} cwd \"$PWD\"\n",
            shell_quote(b["path"].as_str().unwrap()),
            shell_quote(&bin().join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(|st| session(st, id(&s))["cwd"] == b["path"], 5)?;
    ensure!(
        session(&h.state()?, id(&s))["project_id"] == id(&a),
        "cwd moved project ownership"
    );
    let event = json!({"protocol_version":1,"event_id":"fixture-1","terminal_session_id":id(&s),"agent_invocation_id":"fixture-agent","agent_kind":"custom","provider_session_id":"provider-123","state":"waiting_input","request_id":"req-1","sequence":1,"summary":"Fixture needs input","details":"Isolated test","resume":null});
    h.write(
        &mut stream,
        &format!(
            "printf '%s' {} | {} emit\n",
            shell_quote(&event.to_string()),
            shell_quote(&bin().join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(|st| st["notifications"].as_array().unwrap().len() == 1, 5)?;
    h.rpc(json!({"Focus":{"session":id(&s)}}))?;
    let st = h.state()?;
    ensure!(
        st["notifications"][0]["dismissed"] == true && st["agents"][0]["state"] == "waiting_input",
        "Dismissal changed agent state"
    );
    h.rpc(json!({"Hook":event}))?;
    ensure!(
        h.state()?["notifications"].as_array().unwrap().len() == 1,
        "Duplicate hook"
    );
    if let Ok(mut wrong) = h.connect(
        json!({"Cwd":{"session":id(&s),"path":"/"}}),
        Some("wrong"),
        None,
    ) && let Ok(response) = read_frame::<Value>(&mut wrong)
    {
        ensure!(response.get("Error").is_some(), "Invalid auth accepted");
    }
    ensure!(
        session(&h.state()?, id(&s))["cwd"] == b["path"],
        "Invalid auth mutated cwd"
    );
    let layout = json!({"fixture":"layout-does-not-launch-anything"});
    h.rpc(json!({"SaveLayout":{"project":id(&a),"layout":layout}}))?;
    h.write(
        &mut stream,
        "stty -icanon -echo; printf 'BLOCKING_WRITE_READY\\n'; sleep 30\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|t| t.lines().any(|l| l.trim() == "BLOCKING_WRITE_READY"))
        },
        5,
    )?;
    h.write(&mut stream, &"x".repeat(131072))?;
    let start = Instant::now();
    h.rpc(json!({"Stop":{"session":id(&s)}}))?;
    ensure!(
        start.elapsed() < Duration::from_secs(2),
        "Blocked paste prevented Stop"
    );
    h.wait(|st| session(st, id(&s))["lifecycle"] == "ended", 5)?;
    ensure!(
        h.history(id(&s))?.contains("RECONNECT_PROOF_123"),
        "History lost"
    );
    drop(stream);
    h.restart()?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 1 && session(&state, id(&s))["lifecycle"] == "ended",
        "Recovery relaunched a session"
    );
    ensure!(state["projects"][0]["layout"] == layout, "Layout lost");
    let hint = json!({"generation":state["generation"],"revision":state["revision"]});
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint.clone()))?)?
            == "Unchanged",
        "Conditional snapshot changed"
    );
    ensure!(
        h.rpc(json!("Snapshot"))?.get("State").is_some(),
        "Legacy snapshot failed"
    );
    h.project("conditional-snapshot")?;
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint.clone()))?)?
            .get("State")
            .is_some(),
        "Changed snapshot omitted"
    );
    h.restart()?;
    ensure!(
        read_frame::<Value>(&mut h.connect(json!("Snapshot"), None, Some(hint))?)?
            .get("State")
            .is_some(),
        "Restart generation not invalidated"
    );
    println!(
        "{}",
        json!({"functional":"passed","reconnect_same_pid":true,"cwd_grouping":true,"hook_delivery":true,"dedup_and_dismissal":true,"restart_no_launch":true,"conditional_snapshot":true,"legacy_client":true})
    );
    Ok(())
}

pub fn load(duration: Duration, destination: Option<PathBuf>, conditional: bool) -> Result<()> {
    ensure!(
        (1..=3600).contains(&duration.as_secs()),
        "Load duration must be 1–3600 seconds"
    );
    let h = Harness::new()?;
    h.setup()?;
    let projects = (0..5)
        .map(|i| h.project(&format!("load-{i}")))
        .collect::<Result<Vec<_>>>()?;
    let sessions = (0..50)
        .map(|i| h.shell(&projects[i % 5]))
        .collect::<Result<Vec<_>>>()?;
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let mut drains = Vec::new();
    for (i, s) in sessions.iter().take(12).enumerate() {
        let mut stream = h.attach(s)?;
        h.write(&mut stream,"i=0; while [ $i -lt 10000 ]; do printf 'load %s: sample terminal output\\n' \"$i\"; i=$((i+1)); sleep 0.1; done\n")?;
        if i < 6 {
            let stop = stop.clone();
            let bytes = bytes.clone();
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            drains.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match read_frame::<Value>(&mut stream) {
                        Ok(v) => {
                            if let Some(data) = v.get("Data").and_then(Value::as_str)
                                && let Ok(data) = B64.decode(data)
                            {
                                bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }
    }
    let start = Instant::now();
    let mut hint = None::<Value>;
    let mut last_state = h.state()?;
    let mut wire_bytes = 0usize;
    let mut unchanged = 0usize;
    let mut seen = std::collections::HashMap::new();
    let mut history_creations = 0usize;
    let mut history_changes = 0usize;
    let mut times = Vec::new();
    let mut rss = Vec::new();
    while start.elapsed() < duration {
        let t = Instant::now();
        let response: Value = read_frame(&mut h.connect(
            json!("Snapshot"),
            None,
            if conditional { hint.clone() } else { None },
        )?)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        wire_bytes += serde_json::to_vec(&response)?.len() + 4;
        if let Some(state) = response.get("State") {
            last_state = state.clone();
            hint = Some(json!({"revision":state["revision"],"generation":state["generation"]}));
        } else {
            ensure!(response == "Unchanged", "Invalid load snapshot");
            unchanged += 1;
        }
        let state = &last_state;
        for entry in fs::read_dir(h.root.join("history"))? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|e| e != "pty") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let value = (meta.len(), meta.modified().ok());
                match seen.insert(entry.path(), value) {
                    None => history_creations += 1,
                    Some(old) if old != value => history_changes += 1,
                    _ => {}
                }
            }
        }
        ensure!(
            crate::harness::sessions(state)
                .iter()
                .filter(|s| s["lifecycle"] == "running")
                .count()
                >= 50,
            "Load session ended"
        );
        let mut cmd = Command::new("ps");
        cmd.args([
            "-o",
            "rss=",
            "-p",
            &h.daemon.as_ref().unwrap().0.id().to_string(),
        ]);
        if let Ok(raw) = output(cmd)
            && let Ok(value) = String::from_utf8_lossy(&raw).trim().parse::<u64>()
        {
            rss.push(value);
        }
        thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
    }
    stop.store(true, Ordering::Relaxed);
    for task in drains {
        let _ = task.join();
    }
    times.sort_by(f64::total_cmp);
    let report = json!({"duration_seconds":start.elapsed().as_secs_f64(),"conditional":conditional,"snapshot_requests":times.len(),"snapshot_wire_bytes":wire_bytes,"unchanged_responses":unchanged,"snapshot_p50_ms":times[times.len()/2],"history_files_observed":history_creations,"history_metadata_changes_observed":history_changes,"history_activity_scope":"100ms metadata sampling, not syscall counts","sessions":50,"subscribed_streams":6,"gui_rendering_measured":false,"snapshot_p95_ms":times[(times.len()*95/100).min(times.len()-1)],"daemon_peak_rss_kib":rss.iter().max(),"bytes_received":bytes.load(Ordering::Relaxed),"platform":std::env::consts::OS});
    if let Some(path) = destination {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}

pub fn git_shim() -> Result<()> {
    let name = std::env::args_os()
        .next()
        .and_then(|p| {
            PathBuf::from(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .context("Shim program missing")?;
    if name == "gh" {
        let value = std::env::var("TERMINATOR_FIXTURE_GH_JSON")
            .context("The gh fixture requires an explicit JSON payload")?;
        let value: Value = serde_json::from_str(&value)?;
        println!("{value}");
        return Ok(());
    }
    let real = std::env::var_os(format!(
        "TERMINATOR_FIXTURE_REAL_{}",
        name.to_ascii_uppercase()
    ))
    .context("Command shim requires isolated fixture configuration")?;
    let log = std::env::var_os("TERMINATOR_FIXTURE_COMMAND_LOG")
        .context("Missing fixture command log")?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut file = fs::OpenOptions::new().create(true).append(true).open(log)?;
    let record =
        json!({"program":name,"args":args.iter().map(|a|a.to_string_lossy()).collect::<Vec<_>>()});
    file.lock()?;
    file.write_all(format!("{record}\n").as_bytes())?;
    file.unlock()?;
    let status = Command::new(real).args(args).status()?;
    std::process::exit(status.code().unwrap_or(1));
}
pub fn command_counts(seconds: u64, destination: Option<PathBuf>) -> Result<()> {
    ensure!(
        (3..=60).contains(&seconds),
        "Measurement duration must be 3–60 seconds"
    );
    let mut h = Harness::new()?;
    h.setup()?;
    let shim = h.root.join("bin");
    fs::create_dir_all(&shim)?;
    for program in ["git", "ps"] {
        let real =
            terminator_core::find_executable(program).context("Measurement helper missing")?;
        std::os::unix::fs::symlink(std::env::current_exe()?, shim.join(program))?;
        h.env.insert(
            format!("TERMINATOR_FIXTURE_REAL_{}", program.to_ascii_uppercase()),
            real.to_string_lossy().into_owned(),
        );
    }
    let log = h.root.join("commands.jsonl");
    h.env.insert(
        "PATH".into(),
        format!(
            "{}:{}",
            shim.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    h.env.insert(
        "TERMINATOR_FIXTURE_COMMAND_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    let project = h.project("command-counts")?;
    git(
        Path::new(project["path"].as_str().unwrap()),
        &["init", "-q"],
    )?;
    let session = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&session))?;
    let opts = crate::native::Options {
        seconds,
        output: artifacts().join("commands"),
        ..Default::default()
    };
    let mut counts = serde_json::Map::new();
    for visible in [true, false] {
        fs::write(h.root.join("ui-preferences.json"),json!({"version":1,"tool":"Explorer","visible":visible,"typography_migrated":true,"attention_migrated":true}).to_string())?;
        fs::write(&log, "")?;
        crate::native::capture(
            &h,
            &opts,
            if visible { "visible" } else { "hidden" },
            json!([]),
            seconds * 1000,
            |_| Ok(()),
        )?;
        let commands = fs::read_to_string(&log)?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        counts.insert(
            if visible { "visible_git" } else { "hidden_git" }.into(),
            json!(commands.iter().filter(|c| c["program"] == "git").count()),
        );
    }
    fs::write(&log, "")?;
    let mut hook = h.command("terminator-hook");
    hook.args(["event", "codex"])
        .env("TERMINATOR_SESSION_ID", id(&session))
        .env(
            "TERMINATOR_SESSION_TOKEN",
            fs::read_to_string(h.root.join("run/auth"))?.trim(),
        );
    terminator_core::run_command(
        hook,
        terminator_core::CommandOptions {
            input: Some(b"{}".to_vec()),
            ..Default::default()
        },
    )?;
    counts.insert(
        "ancestry_ps".into(),
        json!(
            fs::read_to_string(&log)?
                .lines()
                .filter(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["program"] == "ps"))
                .count()
        ),
    );
    let report = json!({"seconds":seconds,"counts":counts,"scope":"Actual helper invocations in isolated visible/hidden GUI fixtures and one ancestry lookup; selected-context metadata remains active when Explorer is hidden"});
    if let Some(path) = destination {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
pub fn muse(muse: &Path) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("muse-echo")?;
    let s = h.shell(&project)?;
    let mut install = h.command("terminator-hook");
    install
        .args(["install", "muse"])
        .env("TERMINATOR_CONFIG_HOME", project["path"].as_str().unwrap());
    output(install)?;
    let args = vec![
        muse.to_string_lossy().into_owned(),
        "exec".into(),
        "--provider".into(),
        "echo".into(),
        "--workspace".into(),
        project["path"].as_str().unwrap().into(),
        "--no-session-log".into(),
        "--no-foreign-personal-context".into(),
        "--trust-workspace".into(),
        "--disable-web-tools".into(),
        "local hook fixture".into(),
    ];
    h.write(
        &mut h.attach(&s)?,
        &(args
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
            + "\n"),
    )?;
    let state = h.wait(
        |state| {
            state["agents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["kind"] == "muse" && a["state"] == "stopped")
        },
        20,
    )?;
    ensure!(
        state["agents"].as_array().unwrap().len() == 1,
        "Duplicate invocation"
    );
    ensure!(
        state["notifications"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["state"] == "completed"),
        "Missing completion"
    );
    ensure!(
        state["agents"][0]["resume"]["program"] == "muse",
        "Missing resume"
    );
    println!(
        "{}",
        json!({"muse_echo":"passed","invocations":1,"completion_received":true})
    );
    Ok(())
}

/// Additional daemon/CLI features use a fresh instance, including live-session
/// rejection on worktree removal and OSC separation from hook state.
pub fn controls() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("worktree-root")?;
    let repo = PathBuf::from(project["path"].as_str().unwrap());
    git(&repo, &["init", "-q"])?;
    git(&repo, &["config", "user.name", "Fixture"])?;
    git(&repo, &["config", "user.email", "fixture@example.invalid"])?;
    fs::write(repo.join("file"), "base\n")?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-qm", "base"])?;
    let destination = h.root.join("task-checkout");
    let mut command = h.command("terminator-hook");
    command
        .args(["ctl", "worktree", "add", id(&project)])
        .arg(&destination)
        .args(["--branch", "fixture-task"]);
    output(command)?;
    let state = h.state()?;
    let p = state["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == destination.to_string_lossy().as_ref())
        .context("Worktree project not registered")?
        .clone();
    ensure!(
        state["worktrees"].as_array().unwrap().len() == 1,
        "Worktree registry missing"
    );
    let s = h.shell(&p)?;
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Removed a worktree with live sessions"
    );
    let mut send = h.command("terminator-hook");
    send.args([
        "ctl",
        "send",
        id(&s),
        "printf 'CONTROL_SEND_PROOF\\n'",
        "--enter",
    ]);
    output(send)?;
    h.wait(
        |_| {
            h.history(id(&s))
                .is_ok_and(|s| s.contains("CONTROL_SEND_PROOF"))
        },
        5,
    )?;
    let mut read = h.command("terminator-hook");
    read.args(["ctl", "read", id(&s), "--screen"]);
    ensure!(
        String::from_utf8_lossy(&output(read)?).contains("CONTROL_SEND_PROOF"),
        "Screen read did not return live content"
    );
    let mut notify = h.command("terminator-hook");
    notify.args([
        "ctl",
        "notify",
        id(&s),
        "CLI notification",
        "--title",
        "Fixture",
    ]);
    output(notify)?;
    let mut stream = h.attach(&s)?;
    h.write(&mut stream, "printf '\\033]9;OSC fixture\\007'\n")?;
    let state = h.wait(|s| s["terminal_notices"].as_array().unwrap().len() >= 2, 5)?;
    ensure!(
        state["agents"].as_array().unwrap().is_empty()
            && state["notifications"].as_array().unwrap().is_empty(),
        "Terminal notices changed agent lifecycle"
    );
    let notice = state["terminal_notices"][0]["id"].as_str().unwrap();
    let mut dismiss = h.command("terminator-hook");
    dismiss.args(["ctl", "dismiss-notice", notice]);
    output(dismiss)?;
    ensure!(
        h.state()?["terminal_notices"][0]["dismissed"] == true,
        "Notice dismissal failed"
    );
    let stub = h.root.join("gh-fixture");
    fs::create_dir(&stub)?;
    std::os::unix::fs::symlink(std::env::current_exe()?, stub.join("gh"))?;
    let mut metadata = h.command("terminator-hook");
    metadata.args(["ctl", "metadata", id(&s)]);
    metadata.env("PATH",format!("{}:{}",stub.display(),std::env::var("PATH").unwrap_or_default())).env("TERMINATOR_FIXTURE_GH_JSON",json!({"number":7,"title":"Fixture PR","url":"https://github.com/example/fixture/pull/7","state":"OPEN"}).to_string()).arg("--pr");
    let metadata: Value = serde_json::from_slice(&output(metadata)?)?;
    ensure!(
        metadata["branch"] == "fixture-task" && metadata["worktree"] == true,
        "Worktree metadata incorrect: {metadata}"
    );
    ensure!(
        metadata["pull_request"]["number"] == 7,
        "PR metadata fixture failed: {metadata}"
    );
    drop(stream);
    h.rpc(json!({"Stop":{"session":id(&s)}}))?;
    h.wait(|st| session(st, id(&s))["lifecycle"] == "ended", 5)?;
    fs::write(destination.join("dirty"), "keep")?;
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Dirty checkout was removed"
    );
    fs::remove_file(destination.join("dirty"))?;
    // Another project's shell can use this checkout, including after `cd`
    // without a directory hook and in a child whose cwd differs from its shell.
    let outside = h.shell(&project)?;
    let mut outside_stream = h.attach(&outside)?;
    let alias = h.root.join("checkout-alias");
    std::os::unix::fs::symlink(&destination, &alias)?;
    h.write(
        &mut outside_stream,
        &format!(
            "stty -echo; cd {}; printf 'CROSS_PROJECT_READY\\n'\n",
            shell_quote(&alias.to_string_lossy())
        ),
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CROSS_PROJECT_READY"))
        },
        5,
    )?;
    ensure!(
        session(&h.state()?, id(&outside))["cwd"] == project["path"],
        "Fixture unexpectedly reported cwd"
    );
    let error = h
        .rpc(json!({"WorktreeRemove":{"project":id(&p)}}))
        .expect_err("Removed checkout with another project's process inside");
    ensure!(
        error.to_string().contains("Stop processes using"),
        "Wrong cwd refusal: {error:#}"
    );
    ensure!(
        destination.join("file").is_file() && h.state()?["worktrees"][0]["removed"] == false,
        "Refusal changed checkout or registry"
    );
    h.write(
        &mut outside_stream,
        &format!(
            "cd {}; (cd {}; printf 'CHILD_READY\\n'; exec sleep 30) &\n",
            shell_quote(&repo.to_string_lossy()),
            shell_quote(&destination.to_string_lossy())
        ),
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CHILD_READY"))
        },
        5,
    )?;
    let error = h
        .rpc(json!({"WorktreeRemove":{"project":id(&p)}}))
        .expect_err("Removed checkout with a live descendant inside");
    ensure!(
        error.to_string().contains("Stop processes using"),
        "Wrong descendant refusal: {error:#}"
    );
    h.write(
        &mut outside_stream,
        "kill %1; wait; printf 'CHILD_STOPPED\\n'\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&outside))
                .is_ok_and(|s| s.contains("CHILD_STOPPED"))
        },
        5,
    )
    .with_context(|| format!("Child cleanup history: {:?}", h.history(id(&outside))))?;
    h.assert_pids(std::slice::from_ref(&outside))?;
    let recorded = h.rpc(json!({"Create":{"project":id(&project),"cwd":alias,"file":null,"line":null,"editor":false}}))?["Created"].clone();
    ensure!(
        h.rpc(json!({"WorktreeRemove":{"project":id(&p)}})).is_err(),
        "Removed checkout used by another project's recorded cwd"
    );
    h.rpc(json!({"Stop":{"session":id(&recorded)}}))?;
    h.wait(|st| session(st, id(&recorded))["lifecycle"] == "ended", 5)?;
    let branch = git(&repo, &["rev-parse", "refs/heads/fixture-task"])?;
    ensure!(
        git(&destination, &["status", "--porcelain"])?.is_empty(),
        "Lock fixture is dirty"
    );
    git(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "fixture lock",
            destination.to_str().unwrap(),
        ],
    )?;
    let mut locked_remove = h.command("terminator-hook");
    locked_remove.args(["ctl", "worktree", "remove", id(&p)]);
    let error = output(locked_remove).expect_err("Locked checkout was removed");
    ensure!(
        format!("{error:#}").contains("locked"),
        "Wrong refusal: {error:#}"
    );
    ensure!(
        fs::read(destination.join("file"))? == b"base\n",
        "Locked tracked file changed"
    );
    let common = terminator_core::worktrees::common_dir(&repo)?;
    ensure!(
        terminator_core::worktrees::list(&common)?
            .iter()
            .any(|w| w.path == destination && w.locked),
        "Git registration or lock lost"
    );
    ensure!(
        git(&repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
        "Locked branch reference changed"
    );
    let state = h.state()?;
    ensure!(
        state["worktrees"][0]["removed"] == false,
        "Refusal marked checkout removed"
    );
    ensure!(
        session(&state, id(&s))["lifecycle"] == "ended",
        "Lock refusal depended on a live session"
    );
    git(
        &repo,
        &["worktree", "unlock", destination.to_str().unwrap()],
    )?;
    let mut remove = h.command("terminator-hook");
    remove.args(["ctl", "worktree", "remove", id(&p)]);
    output(remove)?;
    h.assert_pids(std::slice::from_ref(&outside))?;
    ensure!(
        !destination.exists() && h.state()?["worktrees"][0]["removed"] == true,
        "Worktree removal not recorded"
    );
    ensure!(
        git(&repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
        "Removal changed branch reference"
    );
    println!(
        "{}",
        json!({"worktree_registry":true,"live_and_dirty_removal_rejected":true,"cross_project_and_descendant_removal_rejected":true,"locked_removal_rejected":true,"branch_preserved":true,"cli_send_read":true,"osc_and_cli_notifications":true,"metadata":true})
    );
    Ok(())
}

pub fn browser_live() -> Result<()> {
    crate::browser_fixture::run()
}
