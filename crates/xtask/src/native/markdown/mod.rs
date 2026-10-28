mod busy;
mod close;
mod modes;

use super::CaptureInput;
use super::{
    Context, Harness, Options, Path, PathBuf, Result, Value, capture, ensure, fs, id, json, output,
    prefs, sessions,
};
pub use busy::busy;
use terminator_core::{Paths, transport, ui_control};

const TEXT: &str = "# Native Markdown\n\nA **native** preview with *emphasis* and [a local file](next.md).\n\n## Features\n\n- [x] Neovim editing\n- [ ] More notes\n\n| Mode | Purpose |\n| --- | --- |\n| Edit | Neovim |\n| Preview | Reading |\n\n```rust\nfn main() { println!(\"Hello\"); }\n```\n\n![Local illustration](<picture space.png>)\n";

/// The Markdown editor under test, its neighbouring shell, and its file.
struct Md<'a> {
    h: &'a Harness,
    o: &'a Options,
    shell: &'a Value,
    editor: &'a Value,
    path: &'a Path,
}

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot).map_err(Into::into)
}
fn expression(h: &Harness, editor: &Value, expression: &str) -> Result<String> {
    let mut command = std::process::Command::new("nvim");
    command
        .arg("--server")
        .arg(transport::nvim_listen_arg(
            &Paths::at(h.root.clone()).editor_socket(id(editor)),
        ))
        .args(["--remote-expr", expression]);
    Ok(String::from_utf8(output(command)?)?.trim().into())
}
fn view(h: &Harness, editor: &Value, text: &str, attached: bool) -> bool {
    gui(h).is_ok_and(|snapshot| {
        let Some(markdown) = snapshot
            .get("markdown")
            .and_then(|markdown| markdown.get(id(editor)))
        else {
            return false;
        };
        markdown["visible"] == true
            && markdown["error"].is_null()
            && markdown["text"].as_str().is_some_and(|t| t.contains(text))
            && snapshot
                .get("visible_terminals")
                .and_then(Value::as_array)
                .is_some_and(|s| s.contains(&editor["id"]) == attached)
    })
}

fn check_header(h: &Harness) -> Result<()> {
    let snapshot = gui(h)?;
    let header = snapshot
        .get("markdown_header")
        .ok_or_else(|| anyhow::anyhow!("missing markdown header"))?;
    let rects = ["title", "edit", "preview", "split", "refresh"].map(|key| &header[key]);
    for rect in &rects {
        ensure!(rect.is_array(), "Missing Markdown header control");
    }
    for pair in rects.windows(2) {
        let left = pair
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing header control"))?;
        let right = pair
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("missing header control"))?;
        ensure!(
            (left[1].as_f64().unwrap() - right[1].as_f64().unwrap()).abs() < 1.0,
            "Markdown header wrapped onto a second row"
        );
        ensure!(
            left[0].as_f64().unwrap() + left[2].as_f64().unwrap()
                <= right[0].as_f64().unwrap() + 1.0,
            "Markdown header controls overlap"
        );
    }
    Ok(())
}

/// A fresh harness with one shell laid out in project `name`; returns the shell and project root.
fn launch(name: &str) -> Result<(Harness, Value, PathBuf)> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project(name)?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let root = PathBuf::from(
        project
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    Ok((h, shell, root))
}

/// Whether the GUI has stored a Markdown mode for `editor`.
fn stored_mode_is_default(h: &Harness, editor: &Value) -> Result<bool> {
    Ok(prefs(h)?
        .get("markdown_modes")
        .ok_or_else(|| anyhow::anyhow!("missing markdown_modes"))?
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("markdown_modes is not an object"))?
        .get(id(editor))
        .is_none_or(Value::is_null))
}

pub fn run(o: &Options) -> Result<()> {
    let (h, shell, root) = launch("markdown")?;
    let path = root.join("read me ' 日本.md");
    write_document(&root, &path)?;
    let editor = default_preview(&h, o)?;
    let md = Md {
        h: &h,
        o,
        shell: &shell,
        editor: &editor,
        path: &path,
    };
    md.split_unsaved()?;
    md.preview()?;
    md.restored_preview()?;
    md.back_to_edit()?;
    md.restored_edit()?;
    md.paused_unsaved_refresh()?;
    md.close_dirty()?;
    md.discard(&root)?;
    h.assert_pids(std::slice::from_ref(&shell))
}

fn write_document(root: &Path, path: &Path) -> Result<()> {
    fs::write(path, TEXT)?;
    fs::write(root.join("next.md"), "# Linked document\n")?;
    image::RgbaImage::from_fn(180, 48, |x, y| {
        let red =
            60u8.saturating_add(u8::try_from(x.checked_div(3).unwrap_or(0)).unwrap_or(u8::MAX));
        let green = 120u8.saturating_add(u8::try_from(y).unwrap_or(u8::MAX));
        image::Rgba([red, green, 210, 255])
    })
    .save(root.join("picture space.png"))?;
    Ok(())
}

/// Open the document from the explorer; returns its editor session.
fn default_preview(h: &Harness, o: &Options) -> Result<Value> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "default-preview",
            actions: json!([
                {"at_ms":1000,"target":"explorer-file:read me ' 日本.md"}
            ]),
            after: 3200,
        },
        |_| {
            h.wait(
                |state| {
                    sessions(state)
                        .iter()
                        .find(|s| s["kind"] == "editor")
                        .is_some_and(|editor| view(h, editor, "# Native Markdown", false))
                },
                8,
            )?;
            check_header(h)
        },
    )?;
    let editor = sessions(&h.state()?)
        .iter()
        .find(|s| s["kind"] == "editor")
        .context("Markdown editor was not opened")?
        .clone();
    ensure!(
        sessions(&h.state()?).len() == 2,
        "Markdown opening duplicated sessions"
    );
    ensure!(
        stored_mode_is_default(h, &editor)?,
        "A newly opened Markdown file must use the default Preview mode"
    );
    Ok(editor)
}
