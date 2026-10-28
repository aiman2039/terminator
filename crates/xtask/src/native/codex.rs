//! Explicitly opt-in live provider fixture. Uses only newly generated public sample text.
use super::{
    Context, Duration, Harness, Options, Path, PathBuf, Process, Result, Value, ensure, fs, id,
    json, root, thread, wait_child,
};
use terminator_core::{Paths, ui_control};

fn snapshot(h: &Harness, sid: &str) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)?
        .get("terminal_scroll")
        .and_then(|scroll| scroll.get(sid))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing terminal scroll"))
}
fn attach(h: &Harness, session: &Value) -> Result<terminator_core::transport::Stream> {
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
    value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("action is not an object"))?
        .insert("capture".into(), json!(true));
    actions.push(value);
    terminator_core::atomic_write(&h.root.join("actions.json"), &serde_json::to_vec(actions)?)?;
    let deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| anyhow::anyhow!("action deadline overflow"))?;
    loop {
        let state = ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)?;
        if state
            .get("fixture_actions_completed")
            .and_then(Value::as_u64)
            == Some(actions.len() as u64)
        {
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
        .env("TERMINATOR_TEST_KEEP_OPEN", "1")
        .env("TERMINATOR_TEST_SIZE", "[1440,800]")
        .env("TERMINATOR_TEST_ACTIONS_PATH", h.root.join("actions.json"))
        .stdout(log.try_clone()?)
        .stderr(log);
    Ok(Process(command.spawn()?))
}
/// One live Codex run: the GUI input stream sent so far and the evidence it produced.
struct Live<'a> {
    h: &'a Harness,
    mode: &'a str,
    idle: &'a Value,
    session: &'a Value,
    target: String,
    actions: Vec<Value>,
    observations: Vec<Value>,
    evidence_path: PathBuf,
}
/// Which terminal mode to exercise, and whether to stop after the focus checks.
struct Mode<'a> {
    mode: &'a str,
    flag: &'a str,
    focus_only: bool,
}
pub fn run(o: &Options) -> Result<()> {
    let codex = terminator_core::find_executable("codex").context("Codex is not installed")?;
    fs::create_dir_all(&o.output)?;
    let focus_only = std::env::var_os("TERMINATOR_TEST_CODEX_FOCUS_ONLY").is_some();
    for (mode, flag) in [("normal", ""), ("no-alt-screen", "--no-alt-screen")] {
        run_mode(
            o,
            &codex,
            &Mode {
                mode,
                flag,
                focus_only,
            },
        )?;
    }
    Ok(())
}
fn run_mode(o: &Options, codex: &Path, m: &Mode<'_>) -> Result<()> {
    let Mode {
        mode,
        flag,
        focus_only,
    } = *m;
    let run_label = format!("{mode}{}", if focus_only { "-focused" } else { "" });
    let evidence_path = o.output.join(format!("{run_label}-evidence.json"));
    let scratch = tempfile::Builder::new()
        .prefix(&format!("codex-scroll-{mode}-"))
        .tempdir_in(root().join("target"))?;
    let h = Harness::new()?;
    h.setup()?;
    let (idle, session) = open_project(&h, scratch.path())?;
    fs::write(h.root.join("actions.json"), "[]")?;
    let mut native = gui(&h, &o.output, &run_label)?;
    wait_for_scroll_rect(&h, &session)?;
    let mut live = Live {
        h: &h,
        mode,
        idle: &idle,
        session: &session,
        target: format!("terminal:{}", id(&session)),
        actions: Vec::new(),
        observations: Vec::new(),
        evidence_path,
    };
    live.preflight()?;
    let mut input = live.launch(codex, flag)?;
    live.focused_history()?;
    live.hover_history()?;
    if focus_only {
        close_gui(&h, &mut native)?;
        h.assert_pids(&[idle, session])?;
        println!("PASS fresh Codex {mode}: focused and hovered history navigation");
        return Ok(());
    }
    let latest_update = live.second_response(&mut input)?;
    close_gui(&h, &mut native)?;
    fs::write(h.root.join("actions.json"), "[]")?;
    let mut native = gui(&h, &o.output, &format!("{mode}-reconnect"))?;
    wait_for_scroll_rect(&h, &session)?;
    live.reconnected(&latest_update)?;
    close_gui(&h, &mut native)?;
    h.assert_pids(&[idle, session])?;
    println!(
        "PASS live Codex {mode}: hover history, focus isolation, new output and GUI reconnect"
    );
    Ok(())
}
/// Register the scratch project and lay out an idle shell beside the Codex shell.
fn open_project(h: &Harness, scratch: &Path) -> Result<(Value, Value)> {
    h.rpc(json!({"AddProject":{"path":scratch}}))?;
    let project = h
        .state()?
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing projects"))?
        .iter()
        .find(|p| p["path"] == scratch.to_string_lossy().as_ref())
        .unwrap()
        .clone();
    let idle = h.shell(&project)?;
    let session = h.shell(&project)?;
    h.layout(&project, &[idle.clone(), session.clone()])?;
    Ok((idle, session))
}
fn wait_for_scroll_rect(h: &Harness, session: &Value) -> Result<()> {
    h.wait(
        |_| {
            snapshot(h, id(session))
                .is_ok_and(|scroll| scroll.get("rect").is_some_and(Value::is_array))
        },
        15,
    )?;
    thread::sleep(Duration::from_secs(1));
    Ok(())
}
fn close_gui(h: &Harness, native: &mut Process) -> Result<()> {
    ui_control::rpc(
        &Paths::at(h.root.clone()),
        ui_control::Request::Window {
            action: "close".into(),
        },
    )?;
    wait_child(&mut native.0, Duration::from_secs(8))?;
    Ok(())
}
/// How long to keep polling, and how often.
struct Poll {
    deadline: std::time::Instant,
    every: Duration,
}
impl Poll {
    fn new(within: Duration, every: Duration) -> Result<Self> {
        let deadline = std::time::Instant::now()
            .checked_add(within)
            .ok_or_else(|| anyhow::anyhow!("codex deadline overflow"))?;
        Ok(Self { deadline, every })
    }
    fn until(
        &self,
        mut done: impl FnMut() -> Result<bool>,
        failure: impl Fn() -> String,
    ) -> Result<()> {
        while !done()? {
            ensure!(std::time::Instant::now() < self.deadline, "{}", failure());
            thread::sleep(self.every);
        }
        Ok(())
    }
}
impl Live<'_> {
    fn act(&mut self, value: Value) -> Result<()> {
        action(self.h, &mut self.actions, value)
    }
    fn snap(&self) -> Result<Value> {
        snapshot(self.h, id(self.session))
    }
    fn scroll(&mut self, times: usize, lines: f64) -> Result<()> {
        for _ in 0..times {
            let target = self.target.clone();
            self.act(json!({"at_ms":0,"target":target,"scroll":lines,"wheel_unit":"line"}))?;
        }
        Ok(())
    }
    fn click_idle(&mut self, hover: bool) -> Result<()> {
        let target = format!("terminal:{}", id(self.idle));
        if hover {
            return self.act(json!({"at_ms":0,"target":target,"hover":true}));
        }
        self.act(json!({"at_ms":0,"target":target}))
    }
    fn save_evidence(&self) -> Result<()> {
        fs::write(
            &self.evidence_path,
            serde_json::to_vec_pretty(&self.observations)?,
        )?;
        Ok(())
    }
    fn preflight(&mut self) -> Result<()> {
        self.click_idle(true)?;
        self.click_idle(false)?;
        let preflight_state = self.snap()?;
        ensure!(
            preflight_state
                .get("focused")
                .is_some_and(|focused| focused == &json!(false)),
            "Fixture cannot deliver a focus click before Codex launch: {preflight_state}"
        );
        // Retain action numbering for the same GUI input stream.
        Ok(())
    }
    /// Start Codex with the sample prompt and wait for its full response.
    fn launch(&self, codex: &Path, flag: &str) -> Result<terminator_core::transport::Stream> {
        let (h, mode) = (self.h, self.mode);
        let prompt = "This is a terminal scrolling validation with public sample data. Do not use tools, read files, or change anything. Output one fenced text block containing exactly 160 lines. Each line starts with TSAMPLE immediately followed by its three-digit line number (1 through 160), then a space and the words public sample. No commentary outside the block.";
        let command = format!(
            "{} --sandbox read-only --ask-for-approval never --disable hooks {flag} {}\r",
            terminator_core::quote(&codex.to_string_lossy()),
            terminator_core::quote(prompt)
        );
        let mut input = attach(h, self.session)?;
        h.write(&mut input, &command)?;
        println!(
            "Live Codex {mode}: started in disposable directory under the already-trusted repository"
        );
        Poll::new(Duration::from_mins(3), Duration::from_millis(250))?.until(
            || Ok(h.history(id(self.session))?.contains("TSAMPLE160")),
            || format!("Live Codex did not produce the sample response in {mode}"),
        )?;
        println!("Live Codex {mode}: sample response generated");
        thread::sleep(Duration::from_secs(1));
        Ok(input)
    }
    fn focused_history(&mut self) -> Result<()> {
        let target = self.target.clone();
        self.act(json!({"at_ms":0,"target":target,"hover":true}))?;
        let latest = self.snap()?;
        self.observations
            .push(json!({"phase":"latest","state":latest}));
        self.act(json!({"at_ms":0,"target":target}))?;
        self.scroll(4, 10.0)?;
        let focused = self.snap()?;
        ensure!(
            focused
                .get("focused")
                .is_some_and(|focused| focused == &json!(true))
                && focused.get("offset").and_then(Value::as_u64).unwrap_or(0) > 0
                && focused
                    .get("samples")
                    .and_then(Value::as_array)
                    .is_some_and(|v| v.iter().any(|n| n.as_u64().is_some_and(|n| n < 120))),
            "Focused Codex history is inaccessible: {focused}"
        );
        self.observations
            .push(json!({"phase":"focused-up","state":focused}));
        self.scroll(6, -10.0)?;
        let focused_latest = self.snap()?;
        ensure!(
            focused_latest
                .get("focused")
                .is_some_and(|focused| focused == &json!(true))
                && focused_latest
                    .get("samples")
                    .and_then(Value::as_array)
                    .is_some_and(|v| v.contains(&json!(160))),
            "Focused scrolling did not return to the latest output: {focused_latest}"
        );
        self.observations
            .push(json!({"phase":"focused-latest","state":focused_latest}));
        Ok(())
    }
    fn hover_history(&mut self) -> Result<()> {
        let mode = self.mode;
        self.click_idle(true)?;
        self.click_idle(false)?;
        for _ in 0..8 {
            self.scroll(1, 10.0)?;
            let state = self.snap()?;
            self.observations
                .push(json!({"phase":"hover-up","state":state}));
        }
        let earlier = self.snap()?;
        self.save_evidence()?;
        ensure!(
            earlier
                .get("focused")
                .is_some_and(|focused| focused == &json!(false)),
            "Hover scrolling changed typing focus"
        );
        ensure!(
            earlier
                .get("samples")
                .and_then(Value::as_array)
                .is_some_and(|v| v.iter().any(|n| n.as_u64().is_some_and(|n| n < 120))),
            "Earlier live Codex messages were not reached in {mode}: {earlier}"
        );
        self.scroll(12, -10.0)?;
        let recent = self.snap()?;
        ensure!(
            recent
                .get("samples")
                .and_then(Value::as_array)
                .is_some_and(|v| v.contains(&json!(160))),
            "Downward scrolling did not reach recent output: {recent}"
        );
        self.observations
            .push(json!({"phase":"back-to-latest","state":recent}));
        self.save_evidence()
    }
    /// Submit a second prompt, observe history while it streams, and wait for
    /// its last line. Returns the final terminal state.
    fn second_response(&mut self, input: &mut terminator_core::transport::Stream) -> Result<Value> {
        let (h, mode) = (self.h, self.mode);
        let second = "Output a fenced text block with exactly 100 more lines. Each starts with TUPDATE immediately followed by its three-digit line number (1 through 100), then a space and public update. Do not use tools or write files. No extra commentary.";
        *input = attach(h, self.session)?;
        h.write(input, &format!("\x1b[200~{second}\x1b[201~"))?;
        thread::sleep(Duration::from_millis(500));
        h.write(input, "\r")?;
        println!("Live Codex {mode}: second prompt submitted separately from paste");
        Poll::new(Duration::from_mins(2), Duration::from_millis(100))?.until(
            || {
                let text = h.history(id(self.session))?;
                Ok(text.contains("TUPDATE001") || text.contains("TUPDATE100"))
            },
            || "Second sample did not start".into(),
        )?;
        println!("Live Codex {mode}: second response started");
        self.during_output()?;
        self.scroll(20, -10.0)?;
        Poll::new(Duration::from_mins(2), Duration::from_millis(250))?.until(
            || {
                Ok(self
                    .snap()?
                    .get("updates")
                    .and_then(Value::as_array)
                    .is_some_and(|updates| updates.contains(&json!(100))))
            },
            || "Latest update was not reachable".into(),
        )?;
        let latest_update = self.snap()?;
        self.observations
            .push(json!({"phase":"latest-update","state":latest_update}));
        Ok(latest_update)
    }
    fn during_output(&mut self) -> Result<()> {
        self.scroll(5, 10.0)?;
        let during = self.snap()?;
        let streamed = markers(&self.h.history(id(self.session))?, "TUPDATE", 100);
        ensure!(
            !streamed.is_empty() && !streamed.contains(&100),
            "Response completed before a live streaming observation"
        );
        ensure!(
            during
                .get("samples")
                .and_then(Value::as_array)
                .is_some_and(|samples| !samples.is_empty()),
            "Earlier conversation was not visible during new output"
        );
        self.observations
            .push(json!({"phase":"during-output","state":during,"daemon_update_markers":streamed}));
        thread::sleep(Duration::from_secs(4));
        let after = self.snap()?;
        ensure!(
            after.get("samples").unwrap_or(&Value::Null)
                == during.get("samples").unwrap_or(&Value::Null),
            "New Codex output moved the retained-history view"
        );
        self.observations
            .push(json!({"phase":"after-output-wait","state":after}));
        Ok(())
    }
    fn reconnected(&mut self, latest_update: &Value) -> Result<()> {
        let reconnected = self.snap()?;
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
            latest_update
                .get("modes")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("missing terminal modes"))?
                & routing_modes
                == reconnected
                    .get("modes")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("missing terminal modes"))?
                    & routing_modes,
            "Terminal input modes changed after reconnect: {latest_update} -> {reconnected}"
        );
        self.observations
            .push(json!({"phase":"reconnected","state":reconnected}));
        self.actions = Vec::new();
        self.click_idle(false)?;
        self.scroll(12, 10.0)?;
        let state = self.snap()?;
        self.observations
            .push(json!({"phase":"reconnected-hover-up","state":state}));
        self.save_evidence()
    }
}
