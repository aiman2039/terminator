use super::super::harness::{Harness, Process, artifacts, id, session_ids, wait_child};
use super::cases_agents::{agent_wheel, agent_wheel_legacy, agent_wheel_live, scrolling};
use super::cases_editor::{cleanup, editor_lifecycle, file_close, focus_close, inline_rename};
use super::cases_gui::{browser, control, external, images, terminal_actions};
use super::cases_startup::{smoke, split_file_opening, workspace_tabs};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Child, time::Duration};
#[derive(Clone)]
pub struct Options {
    pub scale: f32,
    pub narrow: bool,
    pub output: PathBuf,
    pub sessions: usize,
    pub seconds: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            scale: 1.0,
            narrow: false,
            output: artifacts().join("native"),
            sessions: 50,
            seconds: 5,
        }
    }
}
pub(crate) fn capture(
    h: &Harness,
    opts: &Options,
    name: &str,
    actions: Value,
    after: u64,
    during: impl FnOnce(&mut Child) -> Result<()>,
) -> Result<String> {
    let directory = std::path::absolute(&opts.output)?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{name}.png"));
    if path.exists() {
        fs::remove_file(&path)?;
    }
    let logpath = directory.join(format!("{name}.log"));
    let log = fs::File::create(&logpath)?;
    let mut command = h.command("terminator");
    command
        .env("TERMINATOR_CAPTURE_PATH", &path)
        .env("TERMINATOR_CAPTURE_AFTER_MS", after.to_string())
        .env("TERMINATOR_TEST_SCALE", opts.scale.to_string())
        .env("TERMINATOR_TEST_ACTIONS", actions.to_string());
    if opts.narrow {
        command.env("TERMINATOR_TEST_NARROW", "1");
    } else {
        command.env_remove("TERMINATOR_TEST_NARROW");
    }
    if cfg!(target_os = "macos") {
        command
            .env("TERMINATOR_TEST_BACKGROUND", "1")
            .env("TERMINATOR_TEST_VISIBLE_CAPTURE", "1")
            .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1");
    }
    command.stdout(log.try_clone()?).stderr(log);
    let mut gui = Process(command.spawn()?);
    if let Err(error) = during(&mut gui.0) {
        return Err(error.context(fs::read_to_string(&logpath).unwrap_or_default()));
    }
    let status = wait_child(
        &mut gui.0,
        Duration::from_millis(after.saturating_add(20000)),
    )
    .with_context(|| fs::read_to_string(&logpath).unwrap_or_default())?;
    let logs = fs::read_to_string(logpath)?;
    ensure!(status.success(), "GUI failed: {logs}");
    for action in actions.as_array().unwrap() {
        ensure!(
            logs.contains(&format!(
                "Fixture action: {}",
                action["target"].as_str().unwrap()
            )),
            "Action was not reached: {action}; {logs}"
        );
    }
    ensure!(
        logs.contains("Native fixture captured") && path.metadata()?.len() > 1000,
        "Missing fresh native capture"
    );
    Ok(logs)
}
pub(crate) fn plain(
    h: &Harness,
    o: &Options,
    name: &str,
    actions: Value,
    after: u64,
) -> Result<String> {
    capture(h, o, name, actions, after, |_| Ok(()))
}
pub(crate) fn prefs(h: &Harness) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        h.root.join("ui-preferences.json"),
    )?)?)
}
pub(crate) fn save_prefs(h: &Harness, value: &Value) -> Result<()> {
    fs::write(
        h.root.join("ui-preferences.json"),
        serde_json::to_vec_pretty(value)?,
    )?;
    Ok(())
}
pub(crate) fn setup(name: &str) -> Result<(Harness, Value, Vec<Value>, PathBuf)> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project(name)?;
    let root = PathBuf::from(
        p.get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing project path"))?,
    );
    let sessions = (0..6).map(|_| h.shell(&p)).collect::<Result<Vec<_>>>()?;
    h.layout(&p, &sessions)?;
    Ok((h, p, sessions, root))
}
pub fn run(case: &str, opts: Options) -> Result<()> {
    ensure!(
        (1.0..=2.0).contains(&opts.scale),
        "Scale must be between 1 and 2"
    );
    ensure!(
        (3..=60).contains(&opts.seconds) || (case == "responsiveness" && opts.seconds <= 1800),
        "Native duration must be 3–60 seconds"
    );
    let cases = if case == "all" {
        vec![
            "smoke",
            "workspace-tabs",
            "split-file-opening",
            "markdown",
            "markdown-busy",
            "project-sidebar",
            "agent-sidebar",
            "inline-rename",
            "editor-lifecycle",
            "file-close",
            "focus-editor-close",
            "ui-cleanup",
            "external-editor",
            "images",
            "browser",
            "control",
            "terminal-actions",
            "reviews",
            "legacy-diff",
        ]
    } else {
        vec![case]
    };
    for case in cases {
        let mut opts = opts.clone();
        opts.output = opts.output.join(case);
        println!("Running native {case}");
        match case {
            "smoke" => smoke(&opts)?,
            "codex-live" => super::codex::run(&opts)?,
            "launch" => super::launch::run(&opts)?,
            "folder-access" => super::folder_access::run(&opts)?,
            "idle-close" => super::idle_close::run(&opts)?,
            "scrolling" => scrolling(&opts)?,
            "agent-wheel" => agent_wheel(&opts)?,
            "agent-wheel-legacy" => agent_wheel_legacy(&opts)?,
            "agent-wheel-live" => agent_wheel_live(&opts)?,
            "control" => control(&opts)?,
            "workspace-tabs" => workspace_tabs(&opts)?,
            "split-file-opening" => split_file_opening(&opts)?,
            "float-window" => super::float::run(&opts)?,
            "markdown" => super::markdown::run(&opts)?,
            "markdown-busy" => super::markdown::busy(&opts)?,
            "updates" => super::updates::run(&opts)?,
            "installation" => super::installation::run(&opts)?,
            "generations" => super::generations::run(&opts)?,
            "project-sidebar" => super::projects::run(&opts)?,
            "agent-sidebar" => super::agents::run(&opts)?,
            "pane-close" => {
                let (h, _, originals, _) = setup("pane-close")?;
                let first = originals.first().ok_or_else(|| anyhow!("missing pane"))?;
                let target = format!("pane-close:{}", id(first));
                plain(
                    &h,
                    &opts,
                    "confirmation",
                    json!([
                        {"at_ms":1100,"target":target}
                    ]),
                    2500,
                )?;
                h.assert_pids(&originals)?;
                plain(
                    &h,
                    &opts,
                    "backgrounded",
                    json!([
                        {"at_ms":1100,"target":target},
                        {"at_ms":1800,"target":"close-session-keep"}
                    ]),
                    3000,
                )?;
                let state = h.state()?;
                let remaining = session_ids(
                    state
                        .get("projects")
                        .and_then(|projects| projects.get(0))
                        .and_then(|project| project.get("layout"))
                        .ok_or_else(|| anyhow!("missing projects[0].layout"))?,
                );
                ensure!(
                    remaining.len() == originals.len().saturating_sub(1)
                        && !remaining.contains(&id(first).to_owned()),
                    "Pane close removed the wrong session"
                );
                h.assert_pids(&originals)?;
            }
            "popup" => {
                let (h, _, originals, _) = setup("popup")?;
                plain(
                    &h,
                    &opts,
                    "shell-close-dialog",
                    json!([
                        {"at_ms":1100,"target":"workspace-close:Terminal 1"}
                    ]),
                    3000,
                )?;
                h.assert_pids(&originals)?;
            }
            "inline-rename" => inline_rename(&opts)?,
            "editor-lifecycle" => editor_lifecycle(&opts)?,
            "file-close" => file_close(&opts)?,
            "focus-editor-close" => focus_close(&opts)?,
            "ui-cleanup" => cleanup(&opts)?,
            "external-editor" => external(&opts)?,
            "images" => images(&opts)?,
            "responsiveness" => super::responsiveness::run(&opts)?,
            "browser" => browser(&opts)?,
            "terminal-actions" | "ui-flat" | "ui-plan3" => terminal_actions(&opts)?,
            "reviews" => super::reviews::run(&opts)?,
            "legacy-diff" => super::reviews::legacy(&opts)?,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            "window-controls" => super::windows::run(&opts)?,
            #[cfg(windows)]
            "window-controls" => {
                anyhow::bail!("window-controls needs a macOS or X11 desktop driver")
            }
            "renderer-perf" => super::renderer_perf::run(&opts)?,
            "hover-menu" => super::hover_menu::run(&opts)?,
            _ => anyhow::bail!("Unknown native fixture: {case}"),
        }
        println!(
            "{}",
            json!({"fixture":case,"passed":true,"scale":opts.scale,"narrow":opts.narrow,"captures":opts.output})
        );
    }
    Ok(())
}
