use super::super::harness::{Harness, git, id, output, session, session_ids, sessions};
use super::dispatch::{Options, capture, plain, prefs, setup};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, thread, time::Duration};
pub(crate) fn external(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("external-editor")?;
    let source = h.root.join("external-editor/space file.rs");
    fs::write(&source, "fn main() {}\n")?;
    let recorded = h.root.join("arguments.txt");
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .ok_or_else(|| anyhow!("missing settings"))?;
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert("external_editor".into(), json!("/bin/sh"));
    settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings is not an object"))?
        .insert(
            "external_args".into(),
            json!([
                "-c",
                "capture=$1; shift; printf \"%s\\n\" \"$@\" > \"$capture\"; sleep 2; printf \"fixture external exit failure\\n\" >&2; exit 7",
                "fixture",
                recorded,
                "literal argument; $(not evaluated)"
            ]),
        );
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
pub(crate) fn browser(o: &Options) -> Result<()> {
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
    let layout = state
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|project| project.get("layout"))
        .find(|layout| {
            layout["version"] == 6
                || layout.to_string().contains("Browser")
                || layout.to_string().contains("page.html")
        });
    ensure!(
        sessions(&state).len() == 6,
        "HTML allocated an editor: {} sessions, layout {:?}",
        sessions(&state).len(),
        state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .unwrap_or(&Value::Null)
    );
    ensure!(
        layout.is_some_and(|layout| layout["version"] == 6),
        "Browser tab did not persist layout v6: {:?}",
        state.get("projects").unwrap_or(&Value::Null)
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
        h.state()?
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .and_then(|layout| layout.get("tabs"))
            .ok_or_else(|| anyhow!("missing projects[0].layout.tabs"))?
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
pub(crate) fn images(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("images")?;
    let path = root.join("picture.png");
    image::RgbaImage::from_fn(64, 32, |x, y| {
        let red = u8::try_from(x).unwrap_or(u8::MAX).saturating_mul(3);
        let green = u8::try_from(y).unwrap_or(u8::MAX).saturating_mul(7);
        image::Rgba([red, green, 180, 255])
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
        sessions(&state).len() == 6
            && state
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .and_then(|layout| layout.get("version"))
                .is_some_and(|version| version.as_u64() == Some(3)),
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
        h.state()?
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .and_then(|layout| layout.get("tabs"))
            .ok_or_else(|| anyhow!("missing projects[0].layout.tabs"))?
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
pub(crate) fn terminal_actions(o: &Options) -> Result<()> {
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
        prefs(&h)?.get("tool").is_some_and(|tool| tool == "History"),
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
        prefs(&h)?
            .get("history_expanded")
            .and_then(|expanded| expanded.get(id(&p)))
            .is_some_and(|open| open.as_bool() == Some(false)),
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
        prefs(&h)?
            .get("history_expanded")
            .and_then(|expanded| expanded.get(id(&p)))
            .is_some_and(|open| open.as_bool() == Some(true)),
        "History expansion not saved"
    );
    plain(
        &h,
        o,
        "git-open",
        json!([{"at_ms":1000,"target":"tool-Git"},{"at_ms":1900,"target":"git-file-README.md"}]),
        3500,
    )?;
    let git_state = h.state()?;
    let layout = git_state
        .get("projects")
        .and_then(|projects| projects.get(0))
        .and_then(|project| project.get("layout"))
        .ok_or_else(|| anyhow!("missing projects[0].layout"))?
        .to_string();
    ensure!(
        layout.contains("README.md") && layout.contains("Diff"),
        "Git file click did not open a diff: {layout}"
    );
    ensure!(
        sessions(&git_state).iter().all(|s| s["file"]
            .as_str()
            .is_none_or(|path| !path.ends_with("README.md"))),
        "Git file click opened an editor"
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
        sessions(&h.state()?).len() == before.saturating_add(1),
        "Lower pane split failed"
    );
    h.assert_pids(&s)
}

pub(crate) fn control(o: &Options) -> Result<()> {
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
        command.args([
            "ctl",
            "split",
            id(originals
                .first()
                .ok_or_else(|| anyhow!("missing original session"))?),
            "right",
        ]);
        let created: Value = serde_json::from_slice(&output(command)?)?;
        let snapshot = ui_control::rpc(&paths, Request::Snapshot)?;
        ensure!(
            session_ids(
                snapshot
                    .get("workspaces")
                    .and_then(|workspaces| workspaces.get(id(&p)))
                    .ok_or_else(|| anyhow!("missing workspace snapshot"))?,
            )
            .len()
                == 7,
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
                ui_control::rpc(&paths, Request::Snapshot).is_ok_and(|v| {
                    v.get("workspaces")
                        .and_then(|workspaces| workspaces.get(id(&p)))
                        .and_then(|workspace| workspace.get("version"))
                        .is_some_and(|version| version.as_u64() == Some(3))
                })
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
