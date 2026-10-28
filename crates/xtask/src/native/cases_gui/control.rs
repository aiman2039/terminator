use super::super::super::harness::{Harness, id, output, session_ids, sessions};
use super::super::dispatch::{CaptureInput, Options, capture, setup};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::path::Path;
use terminator_core::{
    Paths,
    ui_control::{self, Request},
};

/// The fixture daemon, its GUI control endpoint, and the project under test.
struct Control<'a> {
    h: &'a Harness,
    paths: Paths,
    p: &'a Value,
}

pub(crate) fn control(o: &Options) -> Result<()> {
    let (h, p, originals, root) = setup("gui-control")?;
    image::RgbaImage::from_pixel(24, 12, image::Rgba([70, 140, 210, 255]))
        .save(root.join("ctl.png"))?;
    let actions = json!([]);
    // The window must outlive the GUI round trips: a control request accepted
    // while the window closes never reaches the daemon. The delivery check at
    // the end talks to the daemon only, so it may outlive the window.
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "control",
            actions,
            after: 12000,
        },
        |_| {
            let c = Control {
                h: &h,
                paths: Paths::at(h.root.clone()),
                p: &p,
            };
            h.wait(|_| ui_control::rpc(&c.paths, Request::Ping).is_ok(), 15)?;
            let created = c.split(&originals)?;
            c.open_image(&root)?;
            c.send(&created)
        },
    )?;
    h.assert_pids(&originals)
}

impl Control<'_> {
    /// `ctl split` the first original pane; returns the created session.
    fn split(&self, originals: &[Value]) -> Result<Value> {
        let Self { h, paths, p } = self;
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
        let snapshot = ui_control::rpc(paths, Request::Snapshot)?;
        ensure!(
            session_ids(
                snapshot
                    .get("workspaces")
                    .and_then(|workspaces| workspaces.get(id(p)))
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
        Ok(created)
    }
    fn open_image(&self, root: &Path) -> Result<()> {
        let Self { h, paths, p } = self;
        let mut open = h.command("terminator-hook");
        open.args(["ctl", "open-file", id(p)])
            .arg(root.join("ctl.png"));
        output(open)?;
        h.wait(
            |_| {
                ui_control::rpc(paths, Request::Snapshot).is_ok_and(|v| {
                    v.get("workspaces")
                        .and_then(|workspaces| workspaces.get(id(p)))
                        .and_then(|workspace| workspace.get("version"))
                        .is_some_and(|version| version.as_u64() == Some(3))
                })
            },
            15,
        )?;
        ensure!(
            sessions(&h.state()?).len() == 7,
            "CLI image preview created PTY"
        );
        Ok(())
    }
    // Delivery is a live stream write, so re-send while the proof is
    // missing: a frame lost to a reset connection must not fail the gate.
    // The panes are tiny, so both the echo and the output wrap; compare
    // the history without whitespace. The send splits the marker across
    // shell quotes, so the echoed command cannot contain it and only the
    // command output proves delivery.
    fn send(&self, created: &Value) -> Result<()> {
        let h = self.h;
        let mut delivered = false;
        for _ in 0..3 {
            let mut send = h.command("terminator-hook");
            send.args([
                "ctl",
                "send",
                id(created),
                "printf 'GUI_'CONTROL_''PROOF\\n'",
                "--enter",
            ]);
            output(send)?;
            if h.wait(
                |_| {
                    h.history(id(created)).is_ok_and(|text| {
                        text.split_whitespace()
                            .collect::<String>()
                            .contains("GUI_CONTROL_PROOF")
                    })
                },
                5,
            )
            .is_ok()
            {
                delivered = true;
                break;
            }
        }
        ensure!(
            delivered,
            "CLI send never reached the session: record={:?} history={:?}",
            h.state()?
                .get("sessions")
                .and_then(Value::as_array)
                .and_then(|sessions| sessions.iter().find(|session| session["id"] == id(created))),
            h.history(id(created)).unwrap_or_default()
        );
        Ok(())
    }
}
