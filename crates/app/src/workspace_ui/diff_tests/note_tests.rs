//! Review notes typed in the diff and sent to an agent in one batch.
use super::*;

/// Project `a` with the active working-tree diff of `/a/file.rs`, its
/// first change (line 2) selected.
fn note_diff() -> (App, egui::Context, tempfile::TempDir, Tab) {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, _requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let tab = Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
    };
    let layout = egui_dock::DockState::new(vec![tab.clone()]);
    app.layouts
        .insert("a".into(), Workspace::from_layout(layout));
    let mut doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "same\nold\n",
        right: "same\nnew\n",
        staged: false,
        context: 3,
    });
    doc.options = app.preferences.diff_options;
    let (root, file) = (std::path::Path::new("/a"), std::path::Path::new("file.rs"));
    doc.source = Some(diff::actions::Source::fixture(root, file));
    app.diffs.insert(tab.key(), Ok(doc.into()));
    app.diff_reviews.entry(tab.key()).or_default().selected = Some(0);
    (app, ctx, dir, tab)
}

fn frames(app: &mut App, ctx: &egui::Context, tab: &Tab, frames: Vec<Vec<egui::Event>>) {
    // Time only moves forward across calls.
    let start = ctx.input(|input| input.time) + 1.0;
    let frames: Vec<(f64, Vec<egui::Event>)> = frames
        .into_iter()
        .enumerate()
        .map(|(index, events)| (start + index as f64 * 0.1, events))
        .collect();
    run_diff_frames(RunDiffFramesInput {
        app,
        ctx,
        tab,
        frames: &frames,
    });
}

fn click(app: &mut App, ctx: &egui::Context, tab: &Tab, name: &str) {
    let pos = fixture_target(ctx, name)
        .unwrap_or_else(|| panic!("{name} is painted"))
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frames(
        app,
        ctx,
        tab,
        vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ],
    );
}

fn enter() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_note_on_the_selected_change_joins_the_repository_batch() {
    let (mut app, ctx, _dir, tab) = note_diff();
    frames(&mut app, &ctx, &tab, vec![vec![], vec![]]);
    assert!(fixture_target(&ctx, "diff-send-notes").is_none());
    click(&mut app, &ctx, &tab, "diff-add-note");
    assert!(fixture_target(&ctx, "diff-note-field").is_some());
    frames(
        &mut app,
        &ctx,
        &tab,
        vec![
            vec![egui::Event::Text("use a constant".into())],
            vec![enter()],
            vec![],
        ],
    );
    let notes = app.diff_notes.list(std::path::Path::new("/a"));
    assert_eq!(notes.len(), 1);
    assert_eq!((notes[0].first, notes[0].last), (2, 2));
    assert_eq!(notes[0].text, "use a constant");
    assert_eq!(notes[0].path, std::path::PathBuf::from("/a/file.rs"));
    assert!(fixture_target(&ctx, "diff-send-notes").is_some());
}

#[test]
fn sending_notes_without_a_live_agent_pastes_nothing_and_keeps_them() {
    let (mut app, ctx, _dir, tab) = note_diff();
    let draft = diff::notes::Draft {
        text: "rename".into(),
        ..diff::notes::Draft::new(2, 2, false)
    };
    let note = draft.finish(std::path::Path::new("/a/file.rs")).unwrap();
    app.diff_notes.add(std::path::Path::new("/a"), note);
    frames(&mut app, &ctx, &tab, vec![vec![], vec![]]);
    click(&mut app, &ctx, &tab, "diff-send-notes");
    assert!(app.palette.ask.is_none(), "no agent to pick");
    assert!(app.info.is_some(), "the user sees why");
    assert_eq!(app.diff_notes.list(std::path::Path::new("/a")).len(), 1);
}
