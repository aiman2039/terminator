mod native_rename;
mod split_file_opening;
mod workspace_tabs;

use super::super::harness::{Harness, id, require_first_layout, session_ids};
use super::dispatch::CaptureInput;
use super::dispatch::{Options, plain, prefs, save_prefs};
use anyhow::{Result, anyhow, ensure};
pub(crate) use native_rename::native_rename;
use serde_json::{Value, json};
pub(crate) use split_file_opening::split_file_opening;
use std::{fs, path::Path};
pub(crate) use workspace_tabs::workspace_tabs;

pub(crate) fn smoke(opts: &Options) -> Result<()> {
    ensure!(
        (6..=100).contains(&opts.sessions),
        "Smoke requires 6–100 sessions"
    );
    let mut h = Harness::new()?;
    h.setup()?;
    h.set_setting("font_size", json!(16.0))?;
    let p = h.project("native-ui")?;
    let path = h.root.join("native-ui/hello.rs");
    fs::write(
        &path,
        "fn main() {\n    println!(\"Native Rust terminal\");\n}\n",
    )?;
    let (originals, editor) = open_panes(&h, &p, &path)?;
    for _ in 6..opts.sessions {
        h.shell(&p)?;
    }
    h.layout(&p, &originals)?;
    capture_six_panes(&mut h, opts)?;
    let pref = customize_after_migration(&h, &p)?;
    capture_restart(&h, opts, &pref)?;
    h.assert_pids(&originals)?;
    ensure!(
        session_ids(require_first_layout(&h.state()?)?).len() == 6,
        "Layout lost panes"
    );
    ensure!(
        h.history(id(&editor))?.contains("fn main()")
            && fs::read_to_string(path)?.contains("UI_INSERT"),
        "Native editor input/save failed"
    );
    Ok(())
}

/// Five labelled shells plus the editor; returns all six and the editor.
fn open_panes(h: &Harness, p: &Value, path: &Path) -> Result<(Vec<Value>, Value)> {
    let mut originals = Vec::new();
    for i in 0usize..5 {
        let s = h.shell(p)?;
        h.write(
            &mut h.attach(&s)?,
            &format!(
                "printf '\\033[1;36mSESSION {}\\033[0m\\nNative terminal is connected.\\n'\n",
                i.saturating_add(1)
            ),
        )?;
        originals.push(s);
    }
    let editor = h.editor(p, path)?;
    originals.push(editor.clone());
    Ok((originals, editor))
}

fn capture_six_panes(h: &mut Harness, opts: &Options) -> Result<()> {
    h.env.insert("TERMINATOR_TEST_INPUT".into(), "1".into());
    plain(CaptureInput {
        h,
        opts,
        name: "six-panes",
        actions: json!([]),
        after: opts.seconds.saturating_mul(1000),
    })?;
    h.env.remove("TERMINATOR_TEST_INPUT");
    Ok(())
}

/// Check the typography migration, then save user settings and preferences
/// that a restart must keep. Returns the saved preferences.
fn customize_after_migration(h: &Harness, p: &Value) -> Result<Value> {
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("font_size"))
            .is_some_and(|size| size == &json!(13.0)),
        "Typography migration did not apply"
    );
    let mut pref = prefs(h)?;
    ensure!(
        pref.get("typography_migrated")
            .is_some_and(|migrated| migrated == &json!(true)),
        "Migration marker missing"
    );
    h.set_setting("font_size", json!(18.0))?;
    let preferences = pref
        .as_object_mut()
        .ok_or_else(|| anyhow!("preferences are not an object"))?;
    preferences.insert("tool".into(), json!("git"));
    preferences.insert("visible".into(), json!(false));
    preferences.insert("width".into(), json!(370.0));
    preferences.insert("expanded".into(), json!({id(p):false}));
    save_prefs(h, &pref)?;
    Ok(pref)
}

fn capture_restart(h: &Harness, opts: &Options, pref: &Value) -> Result<()> {
    plain(CaptureInput {
        h,
        opts,
        name: "restart",
        actions: json!([]),
        after: opts.seconds.saturating_mul(1000),
    })?;
    ensure!(
        h.state()?
            .get("settings")
            .and_then(|settings| settings.get("font_size"))
            .is_some_and(|size| size == &json!(18.0))
            && without_ide_strip_docks(&prefs(h)?) == without_ide_strip_docks(pref),
        "User preferences changed on restart"
    );
    Ok(())
}

// The fixture changes sidebar visibility, so the IDE terminal strip's
// painted dock rects legitimately move; they are geometry, not user
// preferences. Compare everything else.
fn without_ide_strip_docks(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(preferences) = value.as_object_mut() {
        preferences.remove("ide_strip_docks");
    }
    value
}
