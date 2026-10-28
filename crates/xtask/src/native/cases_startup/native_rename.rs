use super::super::super::harness::Harness;
use super::super::dispatch::{CaptureInput, Options, plain};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{fs, path::Path, path::PathBuf};

/// Real native editor and rust-analyzer rename through the preview dialog.
/// Uses an isolated project and applies the rename only after the fixture
/// selects the reviewed Apply action.
pub(crate) fn native_rename(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("native-rename")?;
    let root = PathBuf::from(
        p.get("path")
            .and_then(serde_json::Value::as_str)
            .context("project path")?,
    );
    let (helper, main) = write_crate(&root)?;
    let mut settings = h.state()?.get("settings").cloned().context("settings")?;
    settings
        .as_object_mut()
        .context("settings object")?
        .insert("editor_mode".into(), json!("Native"));
    h.rpc(json!({"Settings": settings}))?;
    let shell = h.shell(&p)?;
    h.layout(&p, &[shell])?;
    let files = (helper.as_path(), main.as_path());
    preview_without_apply(&h, o, files)?;
    apply_reviewed_rename(&h, o, files)?;
    capture_native_views(&h, o)
}

/// Write a one-binary crate and a Markdown note; returns `helper.rs` and `main.rs`.
fn write_crate(root: &Path) -> Result<(PathBuf, PathBuf)> {
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[[bin]]\nname = \"probe\"\npath = \"main.rs\"\n",
    )?;
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )?;
    let helper = root.join("helper.rs");
    let main = root.join("main.rs");
    fs::write(&helper, "pub fn helper() {}\n")?;
    fs::write(&main, "mod helper;\nfn main() { helper::helper(); }\n")?;
    fs::write(
        root.join("notes.md"),
        "# Native Markdown\n\n## A readable section\n\n- **Long note:** The native source must wrap this sentence inside the split pane so every word stays visible while the preview keeps a comfortable reading width and clear heading sizes.\n",
    )?;
    Ok((helper, main))
}

fn preview_without_apply(h: &Harness, o: &Options, (helper, main): (&Path, &Path)) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "rename-preview-only",
        actions: json!([
            {"at_ms": 900, "target": "explorer-file:helper.rs"},
            {"at_ms": 1600, "target": "native-file", "hover_offset": [96.0, 10.0]},
            {"at_ms": 1900, "target": "native-file", "key": "F2"},
            {"at_ms": 2300, "target": "rename-name"},
            {"at_ms": 2500, "target": "rename-name", "text": "renamed"},
            {"at_ms": 3000, "target": "rename-preview"},
            {"at_ms": 5000, "target": "rename-apply", "hover": true}
        ]),
        after: 7000,
    })?;
    ensure!(
        fs::read_to_string(helper)?.contains("pub fn helper()")
            && fs::read_to_string(main)?.contains("helper::helper()"),
        "Rename preview changed a file before Apply"
    );
    Ok(())
}

fn apply_reviewed_rename(h: &Harness, o: &Options, (helper, main): (&Path, &Path)) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "reviewed-symbol-rename",
        actions: json!([
            {"at_ms": 1200, "target": "native-file", "hover_offset": [96.0, 10.0]},
            {"at_ms": 1700, "target": "native-file", "key": "F2"},
            {"at_ms": 2100, "target": "rename-name"},
            {"at_ms": 2400, "target": "rename-name", "text": "renamed"},
            {"at_ms": 2900, "target": "rename-preview"},
            {"at_ms": 6500, "target": "rename-apply"},
            {"at_ms": 9000, "target": "explorer-reveal-current"},
            {"at_ms": 9400, "target": "explorer-reveal-active", "hover": true}
        ]),
        after: 12000,
    })?;
    ensure!(
        fs::read_to_string(helper)?.contains("pub fn renamed()"),
        "Definition was not renamed"
    );
    ensure!(
        fs::read_to_string(main)?.contains("helper::renamed()"),
        "Reference was not renamed"
    );
    Ok(())
}

fn capture_native_views(h: &Harness, o: &Options) -> Result<()> {
    plain(CaptureInput {
        h,
        opts: o,
        name: "search-everywhere-symbol",
        actions: json!([
            {"at_ms": 800, "target": "toolbar-search"},
            {"at_ms": 1250, "target": "palette-search", "text": "renamed"},
            {"at_ms": 4500, "target": "palette-item:Symbol  renamed", "hover": true}
        ]),
        after: 7000,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "native-context-menu",
        actions: json!([{"at_ms": 1100, "target": "native-file", "hover_offset": [96.0, 10.0], "right_click": true}]),
        after: 2600,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "native-hover-markdown",
        actions: json!([
            {"at_ms": 3300, "target": "native-file", "hover_offset": [96.0, 10.0], "right_click": true},
            {"at_ms": 3900, "target": "native-hover-action"}
        ]),
        after: 6500,
    })?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "native-markdown-split",
        actions: json!([
            {"at_ms": 900, "target": "explorer-file:notes.md"},
            {"at_ms": 1600, "target": "native-markdown-mode:Split"}
        ]),
        after: 4500,
    })?;
    Ok(())
}
