//! Adjacent file targets must not replace an open terminal file menu.
use super::{Options, capture};
use crate::harness::{Harness, id, sessions};
use anyhow::{Result, ensure};
use serde_json::json;
use std::fs;

pub fn run(opts: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("hover-menu")?;
    let root = h.root.join("hover-menu");
    let first = root.join("first-file.rs");
    let second = root.join("second-file.rs");
    fs::write(&first, "// first file\n")?;
    fs::write(&second, "// second file\n")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let reset = || -> Result<()> {
        h.write(
            &mut h.attach(&shell)?,
            "printf '\\033[2J\\033[Hfirst-file.rs\\nsecond-file.rs\\n'\n",
        )?;
        Ok(())
    };
    let cases = [
        (
            "hover-only",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]},
                {"at_ms":3200,"target":"terminal","hover":true}
            ]),
        ),
        (
            "held-modifier-hover",
            json!([
                {"at_ms":900,"target":"terminal","hover":true,"hover_offset":[500,300],"modifier":"ctrl"},
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]}
            ]),
        ),
        (
            "fixed-target",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":1600,"target":"terminal","hover":true,"modifier":"ctrl"},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]}
            ]),
        ),
        (
            "escape",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":1600,"target":"terminal","hover":true,"modifier":"cmd"},
                {"at_ms":2200,"target":"terminal","hover":true,"key":"Escape"}
            ]),
        ),
        (
            "dismiss",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":1600,"target":"terminal","hover_offset":[45,10]},
                {"at_ms":2200,"target":"terminal-file-menu-close"}
            ]),
        ),
        (
            "outside",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":1600,"target":"terminal","hover_offset":[45,10]},
                {"at_ms":2200,"target":"terminal","hover_offset":[500,300]}
            ]),
        ),
        (
            "open-original-file",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":1600,"target":"terminal","hover_offset":[45,10]},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]},
                {"at_ms":2800,"target":"Open file"}
            ]),
        ),
    ];
    for (name, actions) in cases {
        reset()?;
        let logs = capture(&h, opts, name, actions, 4000, |_| Ok(()))?;
        if matches!(name, "hover-only" | "held-modifier-hover") {
            ensure!(
                !logs.contains("Terminal file menu opened:"),
                "Passive hover opened a menu: {logs}"
            );
            h.assert_pids(std::slice::from_ref(&shell))?;
            continue;
        }
        ensure!(
            logs.matches("Terminal file menu opened:").count() == 1,
            "Menu switched or reopened in {name}: {logs}"
        );
        ensure!(
            logs.contains(&format!("Terminal file menu opened: {}", first.display())),
            "Menu did not keep the original file in {name}: {logs}"
        );
        if name != "fixed-target" {
            ensure!(
                logs.contains("Terminal file menu dismissed:"),
                "Menu did not close in {name}: {logs}"
            );
        }
        h.assert_pids(std::slice::from_ref(&shell))?;
    }

    h.layout(&project, std::slice::from_ref(&shell))?;
    reset()?;
    h.env
        .insert("TERMINATOR_TEST_CAPTURE_COPY".into(), "1".into());
    let logs = capture(
        &h,
        opts,
        "copy-selection",
        json!([
            {"at_ms":900,"target":"terminal","hold":true,"hover_offset":[1,8]},
            {"at_ms":1100,"target":"terminal","hover":true,"hover_offset":[300,8]},
            {"at_ms":1300,"target":"terminal","release_button":true,"hover_offset":[300,8]},
            {"at_ms":1500,"target":"terminal","hover":true,"hover_offset":[45,8]},
            {"at_ms":2000,"target":"terminal","hover":true,"hover_offset":[45,8],"modifier":"ctrl"},
            {"at_ms":2400,"target":"terminal","hover":true,"hover_offset":[45,8],"key":"C","ctrl":true},
            {"at_ms":2800,"target":"terminal","hover":true,"hover_offset":[45,8],"copy":true,"command":true},
            {"at_ms":3100,"target":"terminal","hover":true,"hover_offset":[45,30]},
            {"at_ms":3500,"target":"terminal","hover":true,"hover_offset":[45,30],"key":"C","ctrl":true},
            {"at_ms":3900,"target":"terminal-file-menu-close"},
            {"at_ms":4200,"target":"explorer-search"},
            {"at_ms":4400,"target":"explorer-search","text":"FIELD COPY SAMPLE"},
            {"at_ms":4700,"target":"explorer-search","key":"A","command":true},
            {"at_ms":5000,"target":"explorer-search","copy":true}
        ]),
        5400,
        |_| Ok(()),
    )?;
    ensure!(
        logs.matches("Fixture copied text: \"first-file.rs\"")
            .count()
            == 3,
        "Selected path did not copy through the file menu: {logs}"
    );
    ensure!(
        logs.matches("Fixture copied text: \"FIELD COPY SAMPLE\"")
            .count()
            == 1,
        "Terminal selection took copy away from the focused text field: {logs}"
    );
    ensure!(
        logs.matches("Terminal file menu opened:").count() == 1
            && logs.contains(&format!("Terminal file menu opened: {}", first.display())),
        "Copy or movement replaced the original file menu: {logs}"
    );
    ensure!(
        !h.history(id(&shell))?.contains("^C"),
        "Copy sent an interrupt to the shell"
    );
    h.assert_pids(std::slice::from_ref(&shell))?;

    // The hovered terminal is deliberately rendered after the focused one.
    // Copy must be consumed before that focused terminal can receive Ctrl+C.
    let other = h.shell(&project)?;
    h.layout(&project, &[other.clone(), shell.clone()])?;
    // dash does not dispatch an INT trap at its interactive prompt. Keep a
    // builtin read active so an unconsumed Ctrl+C runs the trap on Linux too.
    h.write(
        &mut h.attach(&other)?,
        "trap 'printf \"COPY_INTERRUPT_REACHED\\n\"' INT; printf '\\033[H\\033[2J\\033[3J'; printf 'HOVER_INPUT_%s\\n' READY; while :; do read -r line; done\n",
    )?;
    h.write(
        &mut h.attach(&shell)?,
        "printf '\\033[2J\\033[Hfirst-file.rs\\nsecond-file.rs\\n'\n",
    )?;
    h.wait(
        |_| {
            h.history(id(&other))
                .is_ok_and(|text| text.contains("HOVER_INPUT_READY"))
        },
        5,
    )?;
    let target = format!("terminal:{}", id(&shell));
    let logs = capture(
        &h,
        opts,
        "copy-hovered-path",
        json!([
            {"at_ms":650,"target":format!("terminal:{}",id(&other))},
            {"at_ms":900,"target":target,"hover":true,"hover_offset":[45,8]},
            {"at_ms":1100,"target":target,"hover":true,"hover_offset":[45,8],"copy":true},
            {"at_ms":1400,"target":target,"hover":true,"hover_offset":[45,8],"modifier":"ctrl"},
            {"at_ms":1700,"target":target,"hover":true,"hover_offset":[45,8],"key":"C","ctrl":true},
            {"at_ms":2000,"target":target,"hover":true,"hover_offset":[45,30]},
            {"at_ms":2400,"target":target,"hover":true,"hover_offset":[45,30],"copy":true},
            {"at_ms":2800,"target":target,"hover":true,"hover_offset":[45,30],"key":"C","ctrl":true},
            {"at_ms":3100,"target":"terminal-file-menu-close"},
            {"at_ms":3500,"target":target,"hover":true,"hover_offset":[300,8]},
            {"at_ms":3900,"target":target,"hover":true,"hover_offset":[300,8],"key":"C","ctrl":true}
        ]),
        4400,
        |_| Ok(()),
    )?;
    let copied = format!("Fixture copied text: {:?}", first.display().to_string());
    ensure!(
        logs.matches(&copied).count() == 4,
        "Hovered path did not copy: {logs}"
    );
    ensure!(
        logs.find(&copied)
            .zip(logs.find("Terminal file menu opened:"))
            .is_some_and(|(copy, menu)| copy < menu),
        "Underlined path did not copy before its menu opened: {logs}"
    );
    ensure!(
        logs.matches("Terminal file menu opened:").count() == 1,
        "Copy replaced the hovered menu: {logs}"
    );
    h.wait(
        |_| {
            h.history(id(&other))
                .is_ok_and(|text| text.contains("COPY_INTERRUPT_REACHED"))
        },
        5,
    )?;
    ensure!(
        h.history(id(&other))?
            .matches("COPY_INTERRUPT_REACHED")
            .count()
            == 1,
        "A hover-copy chord also interrupted the focused shell"
    );
    ensure!(
        !h.history(id(&shell))?.contains("^C"),
        "Copy interrupted the hovered shell"
    );
    h.assert_pids(&[shell.clone(), other])?;

    h.layout(&project, std::slice::from_ref(&shell))?;
    h.write(
        &mut h.attach(&shell)?,
        "printf '\\033[2J\\033[Hhttps://example.com/docs\\n'\n",
    )?;
    let logs = capture(
        &h,
        opts,
        "url-click-menu",
        json!([
            {"at_ms":1000,"target":"terminal","hover":true},
            {"at_ms":1700,"target":"terminal","hover_offset":[45,10]},
            {"at_ms":2200,"target":"terminal","hover":true,"copy":true},
            {"at_ms":2800,"target":"terminal-file-menu-close"}
        ]),
        3500,
        |_| Ok(()),
    )?;
    ensure!(
        logs.matches("Terminal file menu opened: https://example.com/docs")
            .count()
            == 1,
        "URL click did not open its menu: {logs}"
    );
    ensure!(
        logs.contains("Fixture copied text: \"https://example.com/docs\""),
        "URL menu did not preserve copy: {logs}"
    );
    let state = h.wait(
        |state| {
            sessions(state)
                .iter()
                .any(|s| s.get("file").and_then(|v| v.as_str()) == first.to_str())
        },
        5,
    )?;
    ensure!(
        !sessions(&state)
            .iter()
            .any(|s| s.get("file").and_then(|v| v.as_str()) == second.to_str()),
        "Moving into the menu opened the file below it"
    );
    ensure!(
        sessions(&state).iter().any(|s| id(s) == id(&shell)),
        "Shell disappeared"
    );
    Ok(())
}
