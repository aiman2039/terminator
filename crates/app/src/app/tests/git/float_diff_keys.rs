//! Diff-local keys and F7 in a diff tab of a floating window.
use super::*;
use crate::app::center::PaintFloatDockInput;

/// Project `a` whose main dock holds a terminal, and a floating window
/// whose focused pane is a working-tree diff of `/a/file.rs` with two
/// changes, the first selected.
struct FloatDiff {
    app: App,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
    requests: mpsc::Receiver<Job>,
    tab: Tab,
    dock: DockState<Tab>,
    time: f64,
}

fn float_diff() -> FloatDiff {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let mut main = Workspace::from_layout(DockState::new(vec![Tab::Terminal("t".into())]));
    main.main_surface_mut().set_focused_node(NodeIndex::root());
    app.layouts.insert("a".into(), main);
    let tab = Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
    };
    let lines = |first: &str, last: &str| {
        let middle = "same\n".repeat(12);
        format!("{first}\n{middle}{last}\n")
    };
    let mut doc = diff::build_with_context(diff::BuildWithContextInput {
        path: Path::new("file.rs"),
        left: &lines("old", "old"),
        right: &lines("new", "new"),
        staged: false,
        context: 3,
    });
    assert_eq!(doc.hunks.len(), 2);
    doc.options = app.preferences.diff_options;
    doc.source = Some(diff::actions::Source::fixture(
        Path::new("/a"),
        Path::new("file.rs"),
    ));
    app.diffs.insert(tab.key(), Ok(doc.into()));
    app.diff_reviews.entry(tab.key()).or_default().selected = Some(0);
    let mut dock = DockState::new(vec![tab.clone()]);
    dock.main_surface_mut().set_focused_node(NodeIndex::root());
    FloatDiff {
        app,
        ctx,
        _dir: dir,
        requests,
        tab,
        dock,
        time: 1.0,
    }
}

impl FloatDiff {
    /// One frame of the floating window with `events` as its input.
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (app, dock) = (&mut self.app, &mut self.dock);
        let mut output = self.ctx.run_ui(raw, |ui| {
            let _ = app.paint_float_dock(PaintFloatDockInput {
                ui,
                viewport: egui::ViewportId::from_hash_of("float"),
                project: "a",
                dock,
            });
        });
        output.textures_delta.clear();
    }

    fn key(&mut self, event: egui::Event) {
        self.frame(vec![]);
        self.frame(vec![event]);
        self.frame(vec![]);
    }

    fn selected(&self) -> Option<usize> {
        self.app.diff_reviews[&self.tab.key()].selected
    }
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn diff_letter_keys_act_on_the_focused_diff_of_a_floating_window() {
    let mut diff = float_diff();
    diff.key(egui::Event::Text("s".into()));
    let sent: Vec<Job> = diff
        .requests
        .try_iter()
        .filter(|job| matches!(job, Job::DiffAction(..)))
        .collect();
    assert_eq!(sent.len(), 1, "s stages the selected change");
}

#[test]
fn f7_walks_the_changes_of_a_diff_in_a_floating_window() {
    let mut diff = float_diff();
    assert_eq!(diff.selected(), Some(0));
    diff.key(press(egui::Key::F7));
    assert_eq!(diff.selected(), Some(1), "F7 selects the next change");
    diff.key(egui::Event::Text("[".into()));
    diff.key(egui::Event::Text("c".into()));
    assert_eq!(diff.selected(), Some(0), "[c selects the previous change");
}
