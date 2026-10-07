//! Adjacent file targets must not replace an open terminal file menu.
use super::{Options, capture};
use crate::harness::{Harness, id, sessions};
use anyhow::{Result, ensure};
use serde_json::json;
use std::fs;

pub fn run(opts: &Options) -> Result<()> {
    let h = Harness::new()?;
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
            "fixed-target",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]}
            ]),
        ),
        (
            "escape",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover":true,"key":"Escape"}
            ]),
        ),
        (
            "dismiss",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal-file-menu-close"}
            ]),
        ),
        (
            "outside",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover_offset":[500,300]}
            ]),
        ),
        (
            "open-original-file",
            json!([
                {"at_ms":1200,"target":"terminal","hover":true},
                {"at_ms":2200,"target":"terminal","hover":true,"hover_offset":[45,30]},
                {"at_ms":2800,"target":"Open file"}
            ]),
        ),
    ];
    for (name, actions) in cases {
        reset()?;
        let logs = capture(&h, opts, name, actions, 4000, |_| Ok(()))?;
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
