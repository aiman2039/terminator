#[cfg(target_os = "macos")]
use super::macos::{capture_window, float_window_id, pid_windows, windows_named};
#[cfg(target_os = "macos")]
use super::poll;
use super::{DriveInput, Options, drive, group_with, session_label};
use crate::harness::{Harness, id, require_first_layout};
#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Child};

/// Floats one pane through its caption menu, observes the standalone
/// window, closes it through the real dock-back path and asserts the
/// pane returned with its session untouched. `home` is the shell that
/// should keep sharing a tab with the floated pane, or `None` when the
/// floated pane is alone (its tab is dropped: the missing-home path).
pub(super) struct FloatCloseInput<'a> {
    pub(super) h: &'a mut Harness,
    pub(super) o: &'a Options,
    pub(super) name: &'a str,
    pub(super) shells: &'a [Value],
    pub(super) floated: &'a Value,
    pub(super) after: u64,
    pub(super) exit: &'a PathBuf,
}

/// What to look for while the floated window is open, and the marker that closes it.
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        dead_code,
        reason = "Only the macOS observer looks up the window and captures it"
    )
)]
struct FloatWatch<'a> {
    o: &'a Options,
    name: &'a str,
    label: &'a str,
    marker: &'a PathBuf,
}

pub(super) fn float_close(input: FloatCloseInput<'_>) -> Result<()> {
    let FloatCloseInput {
        h,
        o,
        name,
        shells,
        floated,
        after,
        exit,
    } = input;
    let label = session_label(&h.state()?, id(floated))?;
    let marker = o.output.join(format!("{name}-close"));
    let _ = fs::remove_file(&marker);
    // An absent marker file is inert; the fixture creates it once the
    // standalone window has been observed.
    h.env.insert(
        "TERMINATOR_TEST_CLOSE_FLOATING".into(),
        marker.to_string_lossy().into_owned(),
    );
    let caption = format!("pane-caption:{label}");
    let watch = FloatWatch {
        o,
        name,
        label: &label,
        marker: &marker,
    };
    drive(
        DriveInput {
            h,
            o,
            name,
            actions: json!([
                {"at_ms":1500,"target":caption,"right_click":true},
                {"at_ms":1600,"target":"Float window"},
            ]),
            after,
            exit,
        },
        |child| watch.observe(child),
    )?;
    let _ = fs::remove_file(&marker);
    h.assert_pids(shells)?;
    let state = h.state()?;
    let layout = require_first_layout(&state)?;
    ensure!(
        group_with(layout, id(floated)).is_some(),
        "floated pane lost after close"
    );
    Ok(())
}

impl FloatWatch<'_> {
    #[cfg(target_os = "macos")]
    fn observe(&self, child: &mut Child) -> Result<()> {
        let Self {
            o,
            name,
            label,
            marker,
        } = *self;
        let pid = child.id();
        // The standalone window must appear outside the app.
        let output = std::path::absolute(&o.output)?;
        poll(
            || Ok(float_window_id(pid, label).is_some()),
            60,
            &format!(
                "float window never appeared for {label}; pid windows: {:?}; named: {:?}",
                pid_windows(pid),
                windows_named(label)
            ),
        )?;
        let id = float_window_id(pid, label).context("float window lost")?;
        // Best-effort pixels: `screencapture` refuses windows the
        // compositor has no content for yet (occluded fixtures may
        // never paint). Existence, title and dock-back below stay
        // strict; the PNG only illustrates when paint arrives.
        let shot = output.join(format!("{name}-floating.png"));
        let shot_poll = poll(|| Ok(capture_window(id, &shot).is_ok()), 10, "shot");
        match shot_poll {
            Ok(()) => eprintln!("Observed float window {id} for {label}"),
            Err(_) => {
                eprintln!("Observed float window {id} for {label} (unpainted: no pixels captured)")
            }
        }
        // Close through the same dock-back path as the OS close
        // button, then require the window to disappear.
        fs::write(marker, b"close")?;
        poll(
            || Ok(float_window_id(pid, label).is_none()),
            15,
            "float window never docked back",
        )?;
        eprintln!("Float window docked back for {label}");
        Ok(())
    }
    // Linux closes through a marker write with no process lookup.
    #[cfg(not(target_os = "macos"))]
    fn observe(&self, _child: &mut Child) -> Result<()> {
        fs::write(self.marker, b"close")?;
        Ok(())
    }
}
