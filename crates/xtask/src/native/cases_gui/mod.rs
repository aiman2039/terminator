mod control;
mod terminal_actions;

use super::super::harness::{Harness, first_layout, sessions};
use super::dispatch::CaptureInput;
use super::dispatch::{Options, plain, setup};
use anyhow::{Context, Result, anyhow, ensure};
pub(crate) use control::control;
use serde_json::{Value, json};
use std::{fs, path::Path};
pub(crate) use terminal_actions::terminal_actions;

pub(crate) fn external(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("external-editor")?;
    let source = h.root.join("external-editor/space file.rs");
    fs::write(&source, "fn main() {}\n")?;
    let recorded = h.root.join("arguments.txt");
    h.update_settings(|settings| {
        settings.insert("external_editor".into(), json!("/bin/sh"));
        settings.insert(
            "external_args".into(),
            json!([
                "-c",
                "capture=$1; shift; printf \"%s\\n\" \"$@\" > \"$capture\"; sleep 2; printf \"fixture external exit failure\\n\" >&2; exit 7",
                "fixture",
                recorded,
                "literal argument; $(not evaluated)"
            ]),
        );
    })?;
    let s = h.shell(&p)?;
    let logs = plain(CaptureInput {
        h: &h,
        opts: o,
        name: "external-failure",
        actions: json!([{"at_ms":1100,"target":"explorer-file:space file.rs","right_click":true},{"at_ms":1600,"target":"Open externally"},{"at_ms":2100,"target":"header-overflow"},{"at_ms":2400,"target":"settings"}]),
        after: 4900,
    })?;
    let error = logs
        .find("Fixture error External editor exited with")
        .context("Exit failure not displayed")?;
    ensure!(
        logs.contains("exit status: 7")
            && logs.contains("fixture external exit failure")
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

/// Number of top-level tabs in the first project's saved layout.
fn first_tab_count(state: &Value) -> Result<usize> {
    Ok(first_layout(state)
        .and_then(|layout| layout.get("tabs"))
        .ok_or_else(|| anyhow!("missing projects[0].layout.tabs"))?
        .as_array()
        .unwrap()
        .len())
}

pub(crate) fn browser(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("browser")?;
    write_pages(&root)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "html",
        actions: json!([{"at_ms":1100,"target":"explorer-file:page.html"}]),
        after: 3500,
    })?;
    check_browser_tab(&h.state()?)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "browser-close",
        actions: json!([{"at_ms":1100,"target":"workspace-close:Navigation complete"}]),
        after: 3000,
    })?;
    ensure!(
        first_tab_count(&h.state()?)? == 1,
        "Browser close did not remove tab"
    );
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "explicit-text",
        actions: json!([{"at_ms":1100,"target":"explorer-file:page.html","right_click":true},{"at_ms":1600,"target":"Open as text"}]),
        after: 3500,
    })?;
    ensure!(
        sessions(&h.state()?).len() == 7,
        "Open as text did not create an editor"
    );
    h.assert_pids(&s)
}

fn write_pages(root: &Path) -> Result<()> {
    fs::write(
        root.join("page.html"),
        r#"<!doctype html><title>Terminator browser fixture</title><style>body{background:#1d4ed8;color:#fff;font:24px sans-serif;padding:32px}</style><h1 id="ok">loading</h1><script>document.getElementById('ok').textContent='ready';setTimeout(()=>location.href='next.html',200)</script>"#,
    )?;
    fs::write(
        root.join("next.html"),
        "<!doctype html><title>Navigation complete</title><h1>Second page</h1>",
    )?;
    Ok(())
}

/// The HTML file opened a v6 browser tab, not an editor, and followed navigation.
fn check_browser_tab(state: &Value) -> Result<()> {
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
        sessions(state).len() == 6,
        "HTML allocated an editor: {} sessions, layout {:?}",
        sessions(state).len(),
        first_layout(state).unwrap_or(&Value::Null)
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
    Ok(())
}

pub(crate) fn images(o: &Options) -> Result<()> {
    let (h, _, s, root) = setup("images")?;
    let path = root.join("picture.png");
    write_images(&root, &path)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "raster",
        actions: json!([{"at_ms":1100,"target":"explorer-file:picture.png"},{"at_ms":2300,"target":"image-fit"}]),
        after: 3500,
    })?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 6
            && first_layout(&state)
                .and_then(|layout| layout.get("version"))
                .is_some_and(|version| version.as_u64() == Some(3)),
        "Image allocated an editor or failed to version layout"
    );
    restart_and_close_image(&h, o)?;
    open_svg_and_corrupt(&h, o)?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "explicit-text",
        actions: json!([{"at_ms":1100,"target":"explorer-file:picture.png","right_click":true},{"at_ms":1600,"target":"Open as text"}]),
        after: 3500,
    })?;
    ensure!(
        sessions(&h.state()?)
            .iter()
            .any(|s| s["kind"] == "editor" && s["file"] == path.to_string_lossy().as_ref()),
        "Explicit text opening did not use editor"
    );
    h.assert_pids(&s)
}

fn write_images(root: &Path, path: &Path) -> Result<()> {
    image::RgbaImage::from_fn(64, 32, |x, y| {
        let red = u8::try_from(x).unwrap_or(u8::MAX).saturating_mul(3);
        let green = u8::try_from(y).unwrap_or(u8::MAX).saturating_mul(7);
        image::Rgba([red, green, 180, 255])
    })
    .save(path)?;
    fs::write(
        root.join("vector.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80"><rect width="160" height="80" fill="green"/><circle cx="80" cy="40" r="30" fill="orange"/></svg>"#,
    )?;
    fs::write(root.join("broken.png"), "corrupt fixture content")?;
    Ok(())
}

fn restart_and_close_image(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "image-restart",
        actions: json!([{"at_ms":1400,"target":"image-fit"}]),
        after: 2500,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "image-close",
        actions: json!([{"at_ms":1100,"target":"workspace-close:picture.png"}]),
        after: 3000,
    })?;
    ensure!(
        first_tab_count(&h.state()?)? == 1,
        "Image close did not remove tab"
    );
    Ok(())
}

fn open_svg_and_corrupt(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "svg",
        actions: json!([{"at_ms":1100,"target":"explorer-file:vector.svg"},{"at_ms":2400,"target":"image-fit"}]),
        after: 3500,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "corrupt-image",
        actions: json!([{"at_ms":1100,"target":"explorer-file:broken.png"},{"at_ms":2300,"target":"image-error","hover":true}]),
        after: 3300,
    })?;
    ensure!(
        sessions(&h.state()?).len() == 6,
        "Corrupt image opened editor"
    );
    Ok(())
}
