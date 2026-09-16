use crate::harness::*;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Child,
    thread,
    time::Duration,
};
mod agents;
mod codex;
mod folder_access;
mod generations;
mod idle_close;
mod installation;
mod launch;
mod markdown;
mod projects;
mod reviews;
mod updates;
mod windows;

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
pub fn capture(
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
    let logpath = h.root.join(format!("{name}.log"));
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
            .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1");
    }
    command.stdout(log.try_clone()?).stderr(log);
    let mut gui = Process(command.spawn()?);
    if let Err(error) = during(&mut gui.0) {
        return Err(error.context(fs::read_to_string(&logpath).unwrap_or_default()));
    }
    let status = wait_child(&mut gui.0, Duration::from_millis(after + 20000))
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
fn plain(h: &Harness, o: &Options, name: &str, actions: Value, after: u64) -> Result<String> {
    capture(h, o, name, actions, after, |_| Ok(()))
}
fn prefs(h: &Harness) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        h.root.join("ui-preferences.json"),
    )?)?)
}
fn save_prefs(h: &Harness, value: &Value) -> Result<()> {
    fs::write(
        h.root.join("ui-preferences.json"),
        serde_json::to_vec_pretty(value)?,
    )?;
    Ok(())
}
fn setup(name: &str) -> Result<(Harness, Value, Vec<Value>, PathBuf)> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project(name)?;
    let root = PathBuf::from(p["path"].as_str().unwrap());
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
        (3..=60).contains(&opts.seconds),
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
            "codex-live" => codex::run(&opts)?,
            "launch" => launch::run(&opts)?,
            "folder-access" => folder_access::run(&opts)?,
            "idle-close" => idle_close::run(&opts)?,
            "scrolling" => scrolling(&opts)?,
            "control" => control(&opts)?,
            "workspace-tabs" => workspace_tabs(&opts)?,
            "split-file-opening" => split_file_opening(&opts)?,
            "markdown" => markdown::run(&opts)?,
            "markdown-busy" => markdown::busy(&opts)?,
            "updates" => updates::run(&opts)?,
            "installation" => installation::run(&opts)?,
            "generations" => generations::run(&opts)?,
            "project-sidebar" => projects::run(&opts)?,
            "agent-sidebar" => agents::run(&opts)?,
            "pane-close" => {
                let (h, _, originals, _) = setup("pane-close")?;
                let target = format!("pane-close:{}", id(&originals[0]));
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
                let remaining = session_ids(&state["projects"][0]["layout"]);
                ensure!(
                    remaining.len() == originals.len() - 1
                        && !remaining.contains(&id(&originals[0]).to_owned()),
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
            "browser" => browser(&opts)?,
            "terminal-actions" | "ui-flat" | "ui-plan3" => terminal_actions(&opts)?,
            "reviews" => reviews::run(&opts)?,
            "legacy-diff" => reviews::legacy(&opts)?,
            "window-controls" => windows::run(&opts)?,
            _ => anyhow::bail!("Unknown native fixture: {case}"),
        }
        println!(
            "{}",
            json!({"fixture":case,"passed":true,"scale":opts.scale,"narrow":opts.narrow,"captures":opts.output})
        );
    }
    Ok(())
}
fn smoke(opts: &Options) -> Result<()> {
    ensure!(
        (6..=100).contains(&opts.sessions),
        "Smoke requires 6–100 sessions"
    );
    let mut h = Harness::new()?;
    h.setup()?;
    let mut settings = h.state()?["settings"].clone();
    settings["font_size"] = json!(16.0);
    h.rpc(json!({"Settings":settings}))?;
    let p = h.project("native-ui")?;
    let path = h.root.join("native-ui/hello.rs");
    fs::write(
        &path,
        "fn main() {\n    println!(\"Native Rust terminal\");\n}\n",
    )?;
    let mut originals = Vec::new();
    for i in 0..5 {
        let s = h.shell(&p)?;
        h.write(
            &mut h.attach(&s)?,
            &format!(
                "printf '\\033[1;36mSESSION {}\\033[0m\\nNative terminal is connected.\\n'\n",
                i + 1
            ),
        )?;
        originals.push(s);
    }
    let editor = h.editor(&p, &path)?;
    originals.push(editor.clone());
    for _ in 6..opts.sessions {
        h.shell(&p)?;
    }
    h.layout(&p, &originals)?;
    h.env.insert("TERMINATOR_TEST_INPUT".into(), "1".into());
    plain(&h, opts, "six-panes", json!([]), opts.seconds * 1000)?;
    h.env.remove("TERMINATOR_TEST_INPUT");
    ensure!(
        h.state()?["settings"]["font_size"] == 13.0,
        "Typography migration did not apply"
    );
    let mut pref = prefs(&h)?;
    ensure!(
        pref["typography_migrated"] == true,
        "Migration marker missing"
    );
    let mut settings = h.state()?["settings"].clone();
    settings["font_size"] = json!(18.0);
    h.rpc(json!({"Settings":settings}))?;
    pref["tool"] = json!("Git");
    pref["visible"] = json!(false);
    pref["width"] = json!(370.0);
    pref["all_projects"] = json!(true);
    pref["expanded"] = json!({id(&p):false});
    save_prefs(&h, &pref)?;
    plain(&h, opts, "restart", json!([]), opts.seconds * 1000)?;
    ensure!(
        h.state()?["settings"]["font_size"] == 18.0 && prefs(&h)? == pref,
        "User preferences changed on restart"
    );
    h.assert_pids(&originals)?;
    ensure!(
        session_ids(&h.state()?["projects"][0]["layout"]).len() == 6,
        "Layout lost panes"
    );
    ensure!(
        h.history(id(&editor))?.contains("fn main()")
            && fs::read_to_string(path)?.contains("UI_INSERT"),
        "Native editor input/save failed"
    );
    Ok(())
}
fn workspace_tabs(o: &Options) -> Result<()> {
    let (h, _p, originals, root) = setup("workspace-tabs")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    plain(
        &h,
        o,
        "independent-editor",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2100,"target":"workspace-strip","scroll":500.0},{"at_ms":2500,"target":"workspace-tab:Terminal 1"},{"at_ms":3200,"target":"terminal","right_click":true},{"at_ms":3700,"target":"Split right"},{"at_ms":4200,"target":"workspace-strip","scroll":-500.0},{"at_ms":4600,"target":"workspace-tab:source.rs"}]),
        6500,
    )?;
    let state = h.state()?;
    let editor = sessions(&state)
        .iter()
        .find(|s| s["kind"] == "editor")
        .context("Editor not created")?;
    let layout = &state["projects"][0]["layout"];
    ensure!(
        layout["tabs"].as_array().unwrap().len() == 2,
        "File did not open top-level tab"
    );
    let shell_group = layout["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| session_ids(&t["layout"]).contains(&id(&originals[0]).into()))
        .unwrap();
    let editor_group = layout["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| session_ids(&t["layout"]).contains(&id(editor).into()))
        .unwrap();
    ensure!(
        session_ids(&shell_group["layout"]).len() == 7
            && session_ids(&editor_group["layout"]) == [id(editor)],
        "Split ownership changed"
    );
    ensure!(
        layout["active"] == editor_group["id"],
        "Editor not selected"
    );
    plain(
        &h,
        o,
        "restored-shell-layout",
        json!([{"at_ms":700,"target":if o.narrow {"tabs-left"} else {"workspace-tab:Terminal 1"}},{"at_ms":1100,"target":"workspace-tab:Terminal 1"}]),
        3500,
    )?;
    ensure!(
        h.state()?["projects"][0]["layout"]["active"] == shell_group["id"],
        "Selected tab not restored"
    );
    plain(
        &h,
        o,
        "clean-editor-close",
        if o.narrow {
            json!([
                {"at_ms":800,"target":"tabs-right"},
                {"at_ms":1600,"target":"workspace-close:source.rs"}
            ])
        } else {
            json!([{"at_ms":1100,"target":"workspace-close:source.rs"}])
        },
        3500,
    )?;
    h.wait(|s| session(s, id(editor))["lifecycle"] == "ended", 5)?;
    ensure!(
        session_ids(&h.state()?["projects"][0]["layout"]).len() == 7,
        "Closing editor removed shell"
    );
    plain(
        &h,
        o,
        "shell-close-dialog",
        json!([{"at_ms":1100,"target":"workspace-close:Terminal 1"}]),
        3000,
    )?;
    h.assert_pids(&originals)
}
fn split_file_opening(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("original-project")?;
    let original = h.shell(&project)?;
    let old_root = PathBuf::from(project["path"].as_str().unwrap());
    for name in ["clicked.rs", "menu.rs", "split.rs"] {
        fs::write(old_root.join(name), "fn main() {}\n")?;
    }
    let moved_root = h.root.join("moved-project");
    fs::rename(&old_root, &moved_root)?;
    for editor in [false, true] {
        let error = h
            .rpc(json!({"Create":{
                "project":id(&project),"cwd":old_root,
                "file":editor.then(|| old_root.join("clicked.rs")),
                "editor":editor
            }}))
            .expect_err("An unavailable directory must not create a session");
        let message = format!("{error:#}");
        ensure!(
            message.contains(&old_root.to_string_lossy().to_string()) && message.contains("moved"),
            "Creation error must identify the missing directory and recovery: {message}"
        );
    }
    ensure!(
        sessions(&h.state()?).len() == 1,
        "Failed creates changed inventory"
    );
    h.assert_pids(std::slice::from_ref(&original))?;

    // A compatibility path lets the existing daemon, shells and saved layouts
    // survive a move without restarting sessions or rewriting their identities.
    std::os::unix::fs::symlink(&moved_root, &old_root)?;
    for (direction, axis, new_index) in [
        ("up", "Vertical", 1),
        ("down", "Vertical", 2),
        ("left", "Horizontal", 1),
        ("right", "Horizontal", 2),
    ] {
        h.layout(&project, std::slice::from_ref(&original))?;
        let before = sessions(&h.state()?).len();
        plain(
            &h,
            o,
            &format!("split-{direction}"),
            json!([
                {"at_ms":1000,"target":"terminal","right_click":true},
                {"at_ms":1700,"target":format!("Split {direction}")}
            ]),
            3000,
        )?;
        let state = h.state()?;
        ensure!(
            sessions(&state).len() == before + 1,
            "Split {direction} did not create exactly one shell"
        );
        let layout = &state["projects"][0]["layout"];
        ensure!(
            layout["tabs"].as_array().unwrap().len() == 1,
            "Split created a top-level tab"
        );
        let nodes = &layout["tabs"][0]["layout"]["surfaces"][0]["Main"]["nodes"];
        ensure!(
            nodes[0].get(axis).is_some(),
            "Wrong split orientation: {direction}"
        );
        ensure!(
            session_ids(&nodes[3 - new_index]) == [id(&original)],
            "Split {direction} moved the original to the wrong side"
        );
        let created_ids = session_ids(&nodes[new_index]);
        ensure!(
            created_ids.len() == 1 && created_ids[0] != id(&original),
            "New split is missing"
        );
        ensure!(
            session(&state, &created_ids[0])["cwd"] == moved_root.to_string_lossy().as_ref(),
            "Split did not resolve relocated directory"
        );
    }

    for (name, menu, split) in [
        ("clicked.rs", None, false),
        ("menu.rs", Some("Open file"), false),
        ("split.rs", Some("Open in editor split"), true),
    ] {
        h.layout(&project, std::slice::from_ref(&original))?;
        let before = sessions(&h.state()?).len();
        let mut actions = vec![json!({
            "at_ms":1100,"target":format!("explorer-file:{name}"),"right_click":menu.is_some()
        })];
        if let Some(menu) = menu {
            actions.push(json!({"at_ms":1800,"target":menu}));
        } else {
            // A double-click must create one editor, with no duplicate swap-file prompt.
            actions.push(json!({"at_ms":1200,"target":format!("explorer-file:{name}")}));
        }
        plain(&h, o, name, json!(actions), 3300)?;
        let state = h.state()?;
        ensure!(
            sessions(&state).len() == before + 1,
            "Opening {name} did not create exactly one editor"
        );
        let editor = sessions(&state)
            .iter()
            .find(|s| s["file"] == old_root.join(name).to_string_lossy().as_ref())
            .context("Opened file is missing")?;
        ensure!(
            editor["kind"] == "editor" && editor["lifecycle"] == "running",
            "File editor exited"
        );
        ensure!(
            editor["cwd"] == moved_root.to_string_lossy().as_ref(),
            "Editor did not resolve relocated directory"
        );
        let layout = &state["projects"][0]["layout"];
        let tabs = layout["tabs"].as_array().unwrap();
        ensure!(
            tabs.len() == if split { 1 } else { 2 },
            "Wrong file tab placement"
        );
        let active = tabs
            .iter()
            .find(|t| t["id"] == layout["active"])
            .context("Active tab missing")?;
        let ids = session_ids(&active["layout"]);
        ensure!(
            ids.contains(&id(editor).to_owned()) && ids.len() == if split { 2 } else { 1 },
            "Opened editor is not visible in its target tab"
        );
        h.assert_pids(std::slice::from_ref(&original))?;
    }
    Ok(())
}

fn inline_rename(o: &Options) -> Result<()> {
    let (h, _, s, _) = setup("inline-titles")?;
    plain(
        &h,
        o,
        "renamed",
        json!([{"at_ms":1000,"target":"workspace-tab:Terminal 1","right_click":true},{"at_ms":1300,"target":"Rename terminal…"},{"at_ms":1700,"target":"rename-input","text":"Build workspace"},{"at_ms":2000,"target":"rename-input","key":"Enter"},{"at_ms":2500,"target":"terminal","right_click":true},{"at_ms":2800,"target":"Rename terminal…"},{"at_ms":3200,"target":"rename-input","text":"Worker pane"},{"at_ms":3500,"target":"rename-input","key":"Enter"},{"at_ms":4000,"target":format!("session-row:{}",id(&s[1])),"right_click":true},{"at_ms":4300,"target":"Rename terminal…"},{"at_ms":4700,"target":"rename-input","text":"Aux shell"},{"at_ms":5000,"target":"rename-input","key":"Enter"}]),
        6400,
    )?;
    let state = h.state()?;
    for (index, label) in [(0, "Build workspace"), (5, "Worker pane"), (1, "Aux shell")] {
        ensure!(
            session(&state, id(&s[index]))["label"] == label,
            "Rename targeted wrong session"
        );
    }
    plain(
        &h,
        o,
        "editing-inline",
        json!([{"at_ms":900,"target":"workspace-tab:Build workspace","right_click":true},{"at_ms":1200,"target":"Rename terminal…"},{"at_ms":1500,"target":"rename-input","text":"Editing inline"}]),
        2300,
    )?;
    h.assert_pids(&s)
}
fn editor_lifecycle(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("editor-isolation")?;
    fs::write(root.join("source.rs"), "fn main() {}\n")?;
    let actions = json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":5000,"target":format!("session-row:{}",id(&s[5])),"right_click":true},{"at_ms":5400,"target":"Rename terminal…"},{"at_ms":5900,"target":"rename-input","text":"Workspace shell"},{"at_ms":6400,"target":"rename-input","key":"Enter"},{"at_ms":6900,"target":format!("session-row:{}",id(&s[5]))}]);
    capture(&h, o, "editor-lifecycle", actions, 8500, |_| {
        let state = h.wait(|s| sessions(s).iter().any(|s| s["kind"] == "editor"), 5)?;
        let editor = sessions(&state)
            .iter()
            .find(|s| s["kind"] == "editor")
            .unwrap();
        ensure!(
            !s.iter().any(|s| s["pid"] == editor["pid"]),
            "Editor reused shell PTY"
        );
        h.wait(|s| session_ids(&s["projects"][0]["layout"]).len() == 7, 5)?;
        h.write(&mut h.attach(editor)?, ":q\r")?;
        h.wait(|s| session(s, id(editor))["lifecycle"] == "ended", 5)?;
        h.wait(|s| session_ids(&s["projects"][0]["layout"]).len() == 6, 5)?;
        Ok(())
    })?;
    ensure!(
        session(&h.state()?, id(&s[5]))["label"] == "Workspace shell",
        "Sidebar rename failed"
    );
    h.assert_pids(&s)
}
fn file_close(o: &Options) -> Result<()> {
    let (h, _, shells, root) = setup("file-close")?;
    let source = root.join("source.rs");
    fs::write(&source, "fn main() {}\n")?;
    plain(
        &h,
        o,
        "clean-file-close",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":1250,"target":"explorer-file:source.rs"},{"at_ms":2700,"target":"workspace-close:source.rs"}]),
        4000,
    )?;
    let state = h.state()?;
    let editors = sessions(&state)
        .iter()
        .filter(|s| s["kind"] == "editor")
        .collect::<Vec<_>>();
    ensure!(
        editors.len() == 1 && editors[0]["lifecycle"] == "ended",
        "Double click or clean close failed"
    );
    capture(
        &h,
        o,
        "dirty-file-close",
        json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2700,"target":"workspace-close:source.rs"},{"at_ms":3400,"target":"Cancel"},{"at_ms":4100,"target":"workspace-close:source.rs"},{"at_ms":4700,"target":"Save and close"}]),
        6500,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st)
                        .iter()
                        .any(|s| s["kind"] == "editor" && s["lifecycle"] == "running")
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| s["kind"] == "editor" && s["lifecycle"] == "running")
                .unwrap();
            h.write(&mut h.attach(editor)?, "iUnsaved change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r["Text"]
                                .as_str()
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(source)?.contains("Unsaved change"),
        "Save-and-close lost buffer"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state)
            .iter()
            .filter(|s| s["kind"] == "editor")
            .all(|s| s["lifecycle"] == "ended"),
        "Editor did not close"
    );
    ensure!(
        session_ids(&state["projects"][0]["layout"]).len() == 6,
        "Close removed shells"
    );
    let keep = root.join("keep.rs");
    fs::write(&keep, "fn keep() {}\n")?;
    capture(
        &h,
        o,
        "dirty-file-discard",
        json!([
            {"at_ms":1100,"target":"explorer-file:keep.rs"},
            {"at_ms":2700,"target":"workspace-close:keep.rs"},
            {"at_ms":3600,"target":"Discard changes"}
        ]),
        5500,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st).iter().any(|s| {
                        s["kind"] == "editor"
                            && s["lifecycle"] == "running"
                            && s["label"] == "keep.rs"
                    })
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| {
                    s["kind"] == "editor" && s["lifecycle"] == "running" && s["label"] == "keep.rs"
                })
                .unwrap();
            h.write(&mut h.attach(editor)?, "iDiscarded change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r["Text"]
                                .as_str()
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(&keep)? == "fn keep() {}\n",
        "Discard wrote the buffer"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state)
            .iter()
            .filter(|s| s["kind"] == "editor")
            .all(|s| s["lifecycle"] == "ended"),
        "Discard did not close the editor"
    );
    ensure!(
        session_ids(&state["projects"][0]["layout"]).len() == 6,
        "Discard close removed shells"
    );
    let bar = root.join("bar.rs");
    fs::write(&bar, "fn bar() {}\n")?;
    capture(
        &h,
        o,
        "unsaved-close-bar",
        json!([
            {"at_ms":1100,"target":"explorer-file:bar.rs"},
            {"at_ms":2700,"target":"workspace-close:bar.rs"}
        ]),
        3400,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st).iter().any(|s| {
                        s["kind"] == "editor"
                            && s["lifecycle"] == "running"
                            && s["label"] == "bar.rs"
                    })
                },
                5,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| {
                    s["kind"] == "editor" && s["lifecycle"] == "running" && s["label"] == "bar.rs"
                })
                .unwrap();
            h.write(&mut h.attach(editor)?, "iBar change ")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                        .is_ok_and(|r| {
                            r["Text"]
                                .as_str()
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        fs::read_to_string(&bar)? == "fn bar() {}\n",
        "Bar capture wrote the buffer"
    );
    h.assert_pids(&shells)
}
fn focus_close(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("focus-close")?;
    let path = h.root.join("focus-close/source.rs");
    fs::write(&path, "fn main() {}\n")?;
    let mut all = (0..5).map(|_| h.shell(&p)).collect::<Result<Vec<_>>>()?;
    let shells = all.clone();
    let editor = h.editor(&p, &path)?;
    all.push(editor.clone());
    h.layout(&p, &all)?;
    plain(
        &h,
        o,
        "strong-focus",
        json!([{"at_ms":1100,"target":format!("session-row:{}",id(&shells[0]))}]),
        1500,
    )?;
    plain(
        &h,
        o,
        "editor-closed",
        json!([{"at_ms":1100,"target":format!("editor-close:{}",id(&editor))}]),
        3500,
    )?;
    h.wait(|st| session(st, id(&editor))["lifecycle"] == "ended", 5)?;
    ensure!(
        !session_ids(&h.state()?["projects"][0]["layout"]).contains(&id(&editor).into()),
        "Closed editor remains in layout"
    );
    h.assert_pids(&shells)
}
fn cleanup(o: &Options) -> Result<()> {
    let (h, _p, s, root) = setup("ui-cleanup")?;
    let mut settings = h.state()?["settings"].clone();
    settings["external_editor"] = json!("/bin/echo");
    settings["external_args"] = json!(["one argument with spaces"]);
    settings["notifications_side"] = json!(false);
    h.rpc(json!({"Settings":settings}))?;
    fs::write(root.join("modified.rs"), "fn main() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "modified.rs"])?;
    fs::write(root.join("modified.rs"), "fn changed() {}\n")?;
    fs::create_dir(root.join("untracked-folder"))?;
    fs::write(root.join("untracked-folder/new file.rs"), "// new\n")?;
    fs::write(root.join("untracked.txt"), "new\n")?;
    plain(&h, o, "explorer", json!([]), 3500)?;
    ensure!(
        prefs(&h)?["attention_migrated"] == true
            && h.state()?["settings"]["notifications_side"] == true,
        "Attention migration failed"
    );
    plain(
        &h,
        o,
        "overflow",
        Value::Array(
            (0..7)
                .map(|i| json!({"at_ms":1000+i*450,"target":"workspace-plus"}))
                .collect(),
        ),
        5300,
    )?;
    let state = h.state()?;
    let workspace = &state["projects"][0]["layout"];
    let tabs = workspace["tabs"].as_array().unwrap();
    ensure!(
        tabs.len() == 8
            && sessions(&state).len() == 13
            && workspace["active"] == tabs.last().unwrap()["id"],
        "Overflow + failed"
    );
    ensure!(session_ids(workspace).len() == 13, "Overflow lost panes");
    plain(
        &h,
        o,
        "editor-settings",
        json!([{"at_ms":900,"target":"tool-Git"},{"at_ms":1400,"target":"settings"},{"at_ms":2000,"target":"settings-section:Terminal & Editor"},{"at_ms":2300,"target":"external-program"},{"at_ms":2600,"target":"external-program","text":"/draft/editor with spaces"}]),
        4300,
    )?;
    ensure!(
        h.state()?["settings"]["external_editor"] == "/bin/echo"
            && h.state()?["settings"]["external_args"] == json!(["one argument with spaces"]),
        "Unsaved draft changed settings"
    );
    ensure!(prefs(&h)?["tool"] == "Git", "Settings changed sidebar tool");
    let mut settings = h.state()?["settings"].clone();
    settings["notifications_side"] = json!(false);
    h.rpc(json!({"Settings":settings}))?;
    plain(
        &h,
        o,
        "reopen-first-tab",
        json!([{"at_ms":1000,"target":format!("session-row:{}",id(&s[0]))}]),
        3500,
    )?;
    ensure!(
        h.state()?["settings"]["notifications_side"] == false
            && h.state()?["projects"][0]["layout"]["active"] == tabs[0]["id"],
        "Restart lost user placement or selected workspace"
    );
    h.assert_pids(&s)
}
fn external(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("external-editor")?;
    let source = h.root.join("external-editor/space file.rs");
    fs::write(&source, "fn main() {}\n")?;
    let recorded = h.root.join("arguments.txt");
    let mut settings = h.state()?["settings"].clone();
    settings["external_editor"] = json!("/bin/sh");
    settings["external_args"] = json!([
        "-c",
        "capture=$1; shift; printf \"%s\\n\" \"$@\" > \"$capture\"; sleep 2; printf \"fixture external exit failure\\n\" >&2; exit 7",
        "fixture",
        recorded,
        "literal argument; $(not evaluated)"
    ]);
    h.rpc(json!({"Settings":settings}))?;
    let s = h.shell(&p)?;
    let logs = plain(
        &h,
        o,
        "external-failure",
        json!([{"at_ms":1100,"target":"explorer-file:space file.rs","right_click":true},{"at_ms":1600,"target":"Open externally"},{"at_ms":2100,"target":"settings"}]),
        4900,
    )?;
    let error = logs
        .find("Fixture error External editor /bin/sh exited with")
        .context("Exit failure not displayed")?;
    ensure!(
        logs.contains("fixture external exit failure")
            && logs.find("Fixture action: settings").unwrap() < error,
        "Launch blocked Settings or discarded stderr"
    );
    ensure!(
        fs::read_to_string(recorded)?.lines().collect::<Vec<_>>()
            == [
                "literal argument; $(not evaluated)",
                source.to_str().unwrap()
            ],
        "Arguments or path changed"
    );
    h.assert_pids(&[s])
}
fn browser(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("browser")?;
    fs::write(
        root.join("page.html"),
        r#"<!doctype html><title>Terminator browser fixture</title><style>body{background:#1d4ed8;color:#fff;font:24px sans-serif;padding:32px}</style><h1 id="ok">loading</h1><script>document.getElementById('ok').textContent='ready';setTimeout(()=>location.href='next.html',200)</script>"#,
    )?;
    fs::write(
        root.join("next.html"),
        "<!doctype html><title>Navigation complete</title><h1>Second page</h1>",
    )?;
    plain(
        &h,
        o,
        "html",
        json!([{"at_ms":1100,"target":"explorer-file:page.html"}]),
        3500,
    )?;
    let state = h.state()?;
    let layout = state["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|project| &project["layout"])
        .find(|layout| {
            layout["version"] == 6
                || layout.to_string().contains("Browser")
                || layout.to_string().contains("page.html")
        });
    ensure!(
        sessions(&state).len() == 6,
        "HTML allocated an editor: {} sessions, layout {:?}",
        sessions(&state).len(),
        state["projects"][0]["layout"]
    );
    ensure!(
        layout.is_some_and(|layout| layout["version"] == 6),
        "Browser tab did not persist layout v6: {:?}",
        state["projects"]
    );
    ensure!(
        layout.is_some_and(|layout| layout.to_string().contains("next.html")),
        "Native page navigation did not update the saved browser target"
    );
    plain(
        &h,
        o,
        "browser-close",
        json!([{"at_ms":1100,"target":"workspace-close:next.html"}]),
        3000,
    )?;
    ensure!(
        h.state()?["projects"][0]["layout"]["tabs"]
            .as_array()
            .unwrap()
            .len()
            == 1,
        "Browser close did not remove tab"
    );
    plain(
        &h,
        o,
        "explicit-text",
        json!([{"at_ms":1100,"target":"explorer-file:page.html","right_click":true},{"at_ms":1600,"target":"Open as text"}]),
        3500,
    )?;
    ensure!(
        sessions(&h.state()?).len() == 7,
        "Open as text did not create an editor"
    );
    h.assert_pids(&s)
}
fn images(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("images")?;
    let path = root.join("picture.png");
    image::RgbaImage::from_fn(64, 32, |x, y| {
        image::Rgba([x as u8 * 3, y as u8 * 7, 180, 255])
    })
    .save(&path)?;
    fs::write(
        root.join("vector.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80"><rect width="160" height="80" fill="green"/><circle cx="80" cy="40" r="30" fill="orange"/></svg>"#,
    )?;
    fs::write(root.join("broken.png"), "corrupt fixture content")?;
    plain(
        &h,
        o,
        "raster",
        json!([{"at_ms":1100,"target":"explorer-file:picture.png"},{"at_ms":2300,"target":"image-fit"}]),
        3500,
    )?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 6 && state["projects"][0]["layout"]["version"] == 3,
        "Image allocated an editor or failed to version layout"
    );
    plain(
        &h,
        o,
        "image-restart",
        json!([{"at_ms":1400,"target":"image-fit"}]),
        2500,
    )?;
    plain(
        &h,
        o,
        "image-close",
        json!([{"at_ms":1100,"target":"workspace-close:picture.png"}]),
        3000,
    )?;
    ensure!(
        h.state()?["projects"][0]["layout"]["tabs"]
            .as_array()
            .unwrap()
            .len()
            == 1,
        "Image close did not remove tab"
    );
    plain(
        &h,
        o,
        "svg",
        json!([{"at_ms":1100,"target":"explorer-file:vector.svg"},{"at_ms":2400,"target":"image-fit"}]),
        3500,
    )?;
    plain(
        &h,
        o,
        "corrupt-image",
        json!([{"at_ms":1100,"target":"explorer-file:broken.png"},{"at_ms":2300,"target":"image-error","hover":true}]),
        3300,
    )?;
    ensure!(
        sessions(&h.state()?).len() == 6,
        "Corrupt image opened editor"
    );
    plain(
        &h,
        o,
        "explicit-text",
        json!([{"at_ms":1100,"target":"explorer-file:picture.png","right_click":true},{"at_ms":1600,"target":"Open as text"}]),
        3500,
    )?;
    ensure!(
        sessions(&h.state()?)
            .iter()
            .any(|s| s["kind"] == "editor" && s["file"] == path.to_string_lossy().as_ref()),
        "Explicit text opening did not use editor"
    );
    h.assert_pids(&s)
}
fn terminal_actions(o: &Options) -> Result<()> {
    let (h, p, s, root) = setup("terminal-actions")?;
    let source = root.join("hello.rs");
    fs::write(&source, "fn main() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "hello.rs"])?;
    fs::write(&source, "fn changed() {}\n")?;
    fs::write(root.join("README.md"), "untracked\n")?;
    let ended = h.shell(&p)?;
    h.rpc(json!({"Stop":{"session":id(&ended)}}))?;
    h.wait(|st| session(st, id(&ended))["lifecycle"] == "ended", 5)?;
    plain(
        &h,
        o,
        "history",
        json!([{"at_ms":1100,"target":"tool-History"}]),
        2500,
    )?;
    ensure!(
        prefs(&h)?["tool"] == "History",
        "Global History selection not saved"
    );
    plain(
        &h,
        o,
        "history-collapsed",
        json!([
            {"at_ms":1000,"target":format!("history-project:{}",id(&p))}
        ]),
        2000,
    )?;
    ensure!(
        prefs(&h)?["history_expanded"][id(&p)] == false,
        "History collapse not saved"
    );
    plain(
        &h,
        o,
        "history-expanded",
        json!([
            {"at_ms":1000,"target":format!("history-project:{}",id(&p))}
        ]),
        2000,
    )?;
    ensure!(
        prefs(&h)?["history_expanded"][id(&p)] == true,
        "History expansion not saved"
    );
    plain(
        &h,
        o,
        "git-open",
        json!([{"at_ms":1000,"target":"tool-Git"},{"at_ms":1900,"target":"git-file-README.md"}]),
        3500,
    )?;
    ensure!(
        sessions(&h.state()?).iter().any(|s| s["kind"] == "editor"
            && s["file"].as_str().is_some_and(|p| p.ends_with("README.md"))),
        "Git file click did not open editor"
    );
    h.layout(&p, &s)?;
    for session in &s {
        h.write(
            &mut h.attach(session)?,
            "printf '\\033[2J\\033[H./hello.rs:2\\r\\n'\n",
        )?;
        thread::sleep(Duration::from_millis(50));
    }
    plain(
        &h,
        o,
        "hover-editor",
        json!([{"at_ms":1100,"target":"terminal","hover":true},{"at_ms":2400,"target":"Open in editor split"}]),
        4000,
    )?;
    ensure!(
        sessions(&h.state()?)
            .iter()
            .any(|s| s["kind"] == "editor" && s["file"] == source.to_string_lossy().as_ref()),
        "Terminal path hover did not open file"
    );
    let before = sessions(&h.state()?).len();
    plain(
        &h,
        o,
        "lower-split",
        json!([{"at_ms":1100,"target":"terminal","right_click":true},{"at_ms":2000,"target":"Split down"}]),
        3500,
    )?;
    ensure!(
        sessions(&h.state()?).len() == before + 1,
        "Lower pane split failed"
    );
    h.assert_pids(&s)
}

fn control(o: &Options) -> Result<()> {
    use terminator_core::{
        Paths,
        ui_control::{self, Request},
    };
    let (h, p, originals, root) = setup("gui-control")?;
    image::RgbaImage::from_pixel(24, 12, image::Rgba([70, 140, 210, 255]))
        .save(root.join("ctl.png"))?;
    let actions = json!([]);
    capture(&h, o, "control", actions, 5000, |_| {
        let paths = Paths::at(h.root.clone());
        h.wait(|_| ui_control::rpc(&paths, Request::Ping).is_ok(), 5)?;
        let mut command = h.command("terminator-hook");
        command.args(["ctl", "split", id(&originals[0]), "right"]);
        let created: Value = serde_json::from_slice(&output(command)?)?;
        let snapshot = ui_control::rpc(&paths, Request::Snapshot)?;
        ensure!(
            session_ids(&snapshot["workspaces"][id(&p)]).len() == 7,
            "CLI split did not target original layout"
        );
        ensure!(
            sessions(&h.state()?).len() == 7,
            "CLI split created wrong number of sessions"
        );
        let mut send = h.command("terminator-hook");
        send.args([
            "ctl",
            "send",
            id(&created),
            "printf 'GUI_CONTROL_PROOF\\n'",
            "--enter",
        ]);
        output(send)?;
        h.wait(
            |_| {
                h.history(id(&created))
                    .is_ok_and(|t| t.contains("GUI_CONTROL_PROOF"))
            },
            5,
        )?;
        let mut open = h.command("terminator-hook");
        open.args(["ctl", "open-file", id(&p)])
            .arg(root.join("ctl.png"));
        output(open)?;
        h.wait(
            |_| {
                ui_control::rpc(&paths, Request::Snapshot)
                    .is_ok_and(|v| v["workspaces"][id(&p)]["version"] == 3)
            },
            5,
        )?;
        ensure!(
            sessions(&h.state()?).len() == 7,
            "CLI image preview created PTY"
        );
        Ok(())
    })?;
    h.assert_pids(&originals)
}

fn scrolling(opts: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("scrolling-sample")?;
    let left = h.shell(&project)?;
    let right = h.shell(&project)?;
    h.layout(&project, &[left.clone(), right.clone()])?;
    for session in [&left, &right] {
        let mut stream = h.attach(session)?;
        h.write(&mut stream, "i=0; while [ $i -lt 500 ]; do printf 'Sample retained line %s\\n' \"$i\"; i=$((i+1)); done\n")?;
    }
    thread::sleep(Duration::from_millis(500));
    let left_target = format!("terminal:{}", id(&left));
    let right_target = format!("terminal:{}", id(&right));
    let logs = capture(
        &h,
        opts,
        "hover-history",
        json!([
            {"at_ms":900,"target":left_target},
            {"at_ms":1300,"target":right_target,"hover":true},
            {"at_ms":1500,"target":right_target,"scroll":4.0,"wheel_phase":"start"},
            {"at_ms":1600,"target":right_target,"scroll":4.0},
            {"at_ms":1700,"target":right_target,"scroll":4.0},
            {"at_ms":1800,"target":right_target,"scroll":100.0},
            {"at_ms":2200,"target":right_target,"scroll":2.0,"wheel_unit":"line"},
            {"at_ms":2400,"target":right_target,"scroll":0.0,"wheel_phase":"cancel"},
            {"at_ms":3700,"target":right_target,"scroll":-1000.0,"wheel_unit":"line"}
        ]),
        4500,
        |_| {
            thread::sleep(Duration::from_millis(2800));
            h.write(
                &mut h.attach(&right)?,
                "printf 'Sample arriving output\\n'\n",
            )?;
            Ok(())
        },
    )?;
    fs::create_dir_all(&opts.output)?;
    fs::write(opts.output.join("scroll-evidence.log"), &logs)?;
    let prefix = format!(
        "Scroll evidence: session={} focused=false offset=",
        id(&right)
    );
    ensure!(
        logs.lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .any(|s| s
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .is_some_and(|n| n > 0)),
        "Hover-only scrolling did not move unfocused history: {logs}"
    );
    let offsets: Vec<usize> = logs
        .lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter_map(|s| s.split_whitespace().next()?.parse().ok())
        .collect();
    ensure!(
        offsets.last() == Some(&0),
        "Scrolling down did not return to recent output: {logs}"
    );
    ensure!(
        offsets.iter().filter(|offset| **offset > 0).count() >= 2,
        "Missing retained-history observations during output"
    );
    let left_prefix = format!(
        "Scroll evidence: session={} focused=true offset=",
        id(&left)
    );
    ensure!(
        !logs
            .lines()
            .filter_map(|l| l.strip_prefix(&left_prefix))
            .any(|s| !s.starts_with("0 ")),
        "Focused pane scrolled with hovered pane: {logs}"
    );
    h.assert_pids(&[left, right])?;
    Ok(())
}
