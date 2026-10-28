use super::close::{FloatCloseInput, float_close};
use super::{
    DriveInput, Options, dock_leaves, drive, group_with, session_label, tab_key, top_tabs,
};
use crate::harness::{Harness, id, require_first_layout, sessions};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Instant};

/// One fixture harness shared by every float-window scenario.
struct Suite<'a> {
    o: &'a Options,
    t0: Instant,
    h: Harness,
    exit: PathBuf,
    p: Value,
    shells: Vec<Value>,
    first: Value,
}

pub fn run(o: &Options) -> Result<()> {
    let mut suite = Suite::launch(o)?;
    suite.float_home()?;
    suite.float_missing()?;
    let editor = suite.insplit()?;
    suite.stacked_plus()?;
    let labels = suite.stacked_switch(&editor)?;
    suite.detach_dockback(&labels)?;
    suite.drag_detach()?;
    eprintln!(
        "float-window suite done in {:.1}s",
        suite.t0.elapsed().as_secs_f64()
    );
    Ok(())
}

impl<'a> Suite<'a> {
    fn launch(o: &'a Options) -> Result<Self> {
        let t0 = Instant::now();
        let mut h = Harness::new()?;
        h.setup()?;
        // The dispatcher requires an explicit visible desktop for this OS-window case.
        // Shared graceful-exit marker (see `drive`); removed before each
        // scenario so a stale file can never close a fresh fixture.
        let exit = o.output.join("exit-marker");
        h.env.insert(
            "TERMINATOR_TEST_EXIT_MARKER".into(),
            exit.to_string_lossy().into_owned(),
        );
        let p = h.project("float-window")?;
        let shells: Vec<Value> = (0..2).map(|_| h.shell(&p)).collect::<Result<_>>()?;
        let first = shells.first().cloned().context("missing shell")?;
        Ok(Self {
            o,
            t0,
            h,
            exit,
            p,
            shells,
            first,
        })
    }
    /// Options writing captures into `output/<name>`.
    fn opts(&self, name: &str) -> Options {
        let mut opts = self.o.clone();
        opts.output = self.o.output.join(name);
        opts
    }
    fn done(&self, name: &str) {
        eprintln!("{name} done in {:.1}s", self.t0.elapsed().as_secs_f64());
    }
    /// Run a scripted scenario that needs no window observation.
    fn drive(&self, name: &str, actions: Value, after: u64) -> Result<String> {
        drive(
            DriveInput {
                h: &self.h,
                o: &self.opts(name),
                name,
                actions,
                after,
                exit: &self.exit,
            },
            |_| Ok(()),
        )
    }
    // Float from a shared tab: the home tab survives, close docks back.
    fn float_home(&mut self) -> Result<()> {
        self.h.layout(&self.p, &self.shells)?;
        let home_opts = self.opts("float-home");
        float_close(FloatCloseInput {
            h: &mut self.h,
            o: &home_opts,
            name: "float-home",
            shells: &self.shells,
            floated: &self.first,
            after: 30000,
            exit: &self.exit,
        })?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        ensure!(top_tabs(layout)? == 1, "float changed the top-level tabs");
        self.done("float-home");
        Ok(())
    }
    // Float a lone pane: its tab is dropped, close lands in a fresh tab.
    fn float_missing(&mut self) -> Result<()> {
        self.h.layout(&self.p, std::slice::from_ref(&self.first))?;
        let missing_opts = self.opts("float-missing");
        float_close(FloatCloseInput {
            h: &mut self.h,
            o: &missing_opts,
            name: "float-missing",
            shells: &self.shells,
            floated: &self.first,
            after: 25000,
            exit: &self.exit,
        })?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        ensure!(
            top_tabs(layout)? == 1,
            "missing-home dock-back opened a tab"
        );
        ensure!(
            group_with(layout, id(&self.first)).is_some(),
            "floated pane lost after close"
        );
        self.done("float-missing");
        Ok(())
    }
    // Open a file inside the current split instead of a new top-level tab.
    fn insplit(&self) -> Result<Value> {
        let h = &self.h;
        let root = PathBuf::from(
            self.p
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
        );
        fs::write(root.join("note.md"), "# notes\n")?;
        h.layout(&self.p, std::slice::from_ref(&self.first))?;
        let mut stream = h.attach(&self.first)?;
        h.write(&mut stream, "printf '\\033[2J\\033[Hnote.md\\n'\n")?;
        drop(stream);
        let logs = self.drive(
            "insplit",
            json!([
                {"at_ms":1500,"target":"terminal","hover":true},
                {"at_ms":2000,"target":"terminal","hover_offset":[45,10]},
                {"at_ms":2600,"target":"Open in current split"},
            ]),
            20000,
        )?;
        ensure!(
            logs.contains("Terminal file menu opened:"),
            "file menu never opened: {logs}"
        );
        h.assert_pids(&self.shells)?;
        let state = h.state()?;
        let editor = sessions(&state)
            .iter()
            .find(|s| s["kind"] == "editor")
            .cloned()
            .context("no editor session created")?;
        let layout = require_first_layout(&state)?;
        ensure!(
            top_tabs(layout)? == 1,
            "in-split open created a top-level tab"
        );
        ensure!(
            dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 2),
            "in-split open did not stack: {layout}"
        );
        self.done("insplit");
        Ok(editor)
    }
    // Leaf + stacks a third tab into the same split, no new top-level tab.
    fn stacked_plus(&self) -> Result<()> {
        self.drive(
            "stacked-plus",
            json!([{"at_ms":1500,"target":"pane-plus"}]),
            14000,
        )?;
        self.h.assert_pids(&self.shells)?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        ensure!(top_tabs(layout)? == 1, "leaf + opened a top-level tab");
        ensure!(
            dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 3),
            "leaf + did not stack: {layout}"
        );
        self.done("stacked-plus");
        Ok(())
    }
    // Clicking a stacked tab switches to it. Returns the shell and editor labels.
    fn stacked_switch(&self, editor: &Value) -> Result<(String, String)> {
        let state = self.h.state()?;
        let editor_sid = id(editor).to_owned();
        let shell_label = session_label(&state, id(&self.first))?;
        let editor_label = session_label(&state, &editor_sid)?;
        self.drive(
            "stacked-switch",
            json!([{"at_ms":1500,"target":format!("leaf-tab:{}",tab_key(&editor_sid))}]),
            12000,
        )?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        let switched = dock_leaves(layout).iter().any(|(tabs, active)| {
            tabs.iter().position(|tab| {
                tab.get("Terminal").and_then(Value::as_str) == Some(editor_sid.as_str())
            }) == Some(*active)
        });
        ensure!(
            switched,
            "stacked switch did not select the editor: {layout}"
        );
        self.done("stacked-switch");
        Ok((shell_label, editor_label))
    }
    // Detach to a new tab, then dock back through the submenu.
    fn detach_dockback(&self, (shell_label, editor_label): &(String, String)) -> Result<()> {
        self.drive(
            "detach-dockback",
            json!([
                {"at_ms":1000,"target":format!("leaf-tab:{}", tab_key(id(&self.first)))},
                {"at_ms":1800,"target":format!("pane-caption:{shell_label}"),"right_click":true},
                {"at_ms":2600,"target":"Detach to new tab"},
                {"at_ms":4400,"target":format!("pane-caption:{shell_label}"),"right_click":true},
                {"at_ms":5200,"target":"Move to tab","hover":true},
                {"at_ms":6000,"target":editor_label},
            ]),
            16000,
        )?;
        self.h.assert_pids(&self.shells)?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        ensure!(
            top_tabs(layout)? == 1,
            "dock-back left extra tabs: {layout}"
        );
        ensure!(
            dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 3),
            "dock-back did not restore the stack: {layout}"
        );
        self.done("detach-dockback");
        Ok(())
    }
    // Real pane drag to the strip +: press-hold, moves, release.
    fn drag_detach(&self) -> Result<()> {
        self.h.layout(&self.p, &self.shells)?;
        let state = self.h.state()?;
        let shell_label = session_label(&state, id(&self.first))?;
        self.drive(
            "drag-detach",
            json!([
                {"at_ms":1500,"target":format!("pane-caption:{shell_label}"),"hold":true},
                {"at_ms":2300,"target":"workspace-plus","hover":true},
                {"at_ms":3000,"target":"workspace-plus","hover":true},
                {"at_ms":3700,"target":"workspace-plus","release_button":true},
            ]),
            12000,
        )?;
        self.h.assert_pids(&self.shells)?;
        let state = self.h.state()?;
        let layout = require_first_layout(&state)?;
        ensure!(top_tabs(layout)? == 2, "drag did not detach: {layout}");
        ensure!(
            group_with(layout, id(&self.first)).is_some(),
            "dragged pane lost"
        );
        self.done("drag-detach");
        Ok(())
    }
}
