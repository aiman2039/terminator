//! Queued editor commands (Find Action) do what their chords do in a
//! focused view, run once in their own view, and respect read-only and
//! vim buffers.
use crate::doc::{Buffer, Doc};
use crate::highlight::{HighlightCache, Language};
use crate::lsp::Severity;
use crate::view::command::queue_id;
use crate::view::{
    EditorCommand, ShowSourceInput, SourceOptions, Squiggle, chord_label, queue_command,
    queued_command, show_source, source_focus_id,
};
use crate::vim::{Cursor, Effect, ModalEngine, Mode, VimEngine};
use std::time::{Duration, Instant};

const TEXT: &str = "fn main() {\n    let a = 1;\n    let a = 2;\n}\n";
const ID: &str = "command-view";
const MAC: bool = cfg!(target_os = "macos");

/// What a run left behind, compared between a chord and its command.
#[derive(Debug, PartialEq)]
struct Snapshot {
    text: String,
    cursor: Cursor,
    selection: Option<(Cursor, Cursor)>,
    mode: Mode,
    carets: usize,
    effects: Vec<Effect>,
    copied: Vec<String>,
    rows: usize,
}

/// The second frame's input.
enum Press {
    Idle,
    Chord(egui::Key, egui::Modifiers),
    Queue(EditorCommand),
}

struct Editor {
    ctx: egui::Context,
    doc: Doc,
    engine: VimEngine,
    highlight: HighlightCache,
    options: SourceOptions,
}

impl Editor {
    /// A modeless Rust buffer with an undo step, the caret on `a` in line
    /// 2, and one problem on line 3.
    fn new(jetbrains: bool) -> Self {
        let mut doc = Doc::new(TEXT);
        doc.begin_undo_group();
        doc.insert(doc.len_chars(), "// end\n");
        doc.end_undo_group();
        let mut engine = VimEngine::new();
        let _ = engine.place_cursor(&mut doc, 1, 8);
        let problem = Squiggle {
            line: 2,
            start: 8,
            end: 9,
            severity: Severity::Warning,
            message: "unused".into(),
        };
        Self {
            ctx: egui::Context::default(),
            doc,
            engine,
            highlight: HighlightCache::default(),
            options: SourceOptions {
                vim: false,
                jetbrains_keys: jetbrains,
                reference: Some("src/main.rs".into()),
                squiggles: std::sync::Arc::new(vec![problem]),
                ..SourceOptions::default()
            },
        }
    }

    /// One frame with `events`: its effects, copied text and row count.
    fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<Effect>, Vec<String>, usize) {
        let (mut effects, mut rows) = (Vec::new(), 0);
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let outcome = show_source(ShowSourceInput {
                    ui,
                    doc: &mut self.doc,
                    engine: &mut self.engine,
                    options: &self.options,
                    highlight: &mut self.highlight,
                    language: Language::Rust,
                    id: ID,
                });
                effects.extend(outcome.effects);
                rows = outcome.visual_rows;
            },
        );
        output.textures_delta.clear();
        let copied = output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        (effects, copied, rows)
    }

    /// Focus frame, then the press, then a frame that lays out its result.
    fn run(mut self, press: Press) -> Snapshot {
        self.frame(Vec::new());
        let events = match press {
            Press::Idle => Vec::new(),
            Press::Chord(key, modifiers) => vec![key_event(key, modifiers)],
            Press::Queue(command) => {
                queue_command(&self.ctx, ID, command);
                Vec::new()
            }
        };
        let (effects, copied, _) = self.frame(events);
        let (_, _, rows) = self.frame(Vec::new());
        Snapshot {
            text: self.doc.text(),
            cursor: self.engine.cursor(),
            selection: self.engine.selection(),
            mode: self.engine.mode(),
            carets: self.engine.extra_carets().len(),
            effects,
            copied,
            rows,
        }
    }
}

fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn queued_commands_do_what_their_chords_do() {
    // No-ops on this buffer: nothing to shrink, unselect, unfold or redo.
    let idle_here = [
        EditorCommand::ShrinkSelection,
        EditorCommand::UnselectOccurrence,
        EditorCommand::Unfold,
        EditorCommand::UnfoldAll,
        EditorCommand::Redo,
    ];
    for jetbrains in [true, false] {
        let idle = Editor::new(jetbrains).run(Press::Idle);
        for &command in EditorCommand::ALL {
            let Some((key, modifiers)) = command.chord(MAC, jetbrains) else {
                continue;
            };
            let label = chord_label(key, modifiers, MAC);
            let queued = Editor::new(jetbrains).run(Press::Queue(command));
            let pressed = Editor::new(jetbrains).run(Press::Chord(key, modifiers));
            assert_eq!(
                pressed, queued,
                "{command:?}: {label} and the queued command differ (jetbrains {jetbrains})"
            );
            assert_eq!(
                pressed == idle,
                idle_here.contains(&command),
                "{command:?}: {label} must act on this buffer (jetbrains {jetbrains})"
            );
        }
    }
}

#[test]
fn commands_without_a_chord_still_run_from_the_queue() {
    let types = Editor::new(true).run(Press::Queue(EditorCommand::TypeHierarchy));
    assert_eq!(types.effects, [Effect::Hierarchy { types: true }]);
    assert_eq!(EditorCommand::GotoLine.chord(MAC, false), None);
    let line = Editor::new(false).run(Press::Queue(EditorCommand::GotoLine));
    assert_eq!(line.mode, Mode::Command);
}

#[test]
fn chord_labels_match_the_editor_menu() {
    let label = |command: EditorCommand, mac: bool, jetbrains: bool| {
        command
            .chord(mac, jetbrains)
            .map(|(key, modifiers)| chord_label(key, modifiers, mac))
    };
    let mac = [
        (EditorCommand::DuplicateLine, "⌘D"),
        (EditorCommand::DeleteLine, "⌘⌫"),
        (EditorCommand::MoveLineUp, "⌥⇧↑"),
        (EditorCommand::JoinLines, "⌃⇧J"),
        (EditorCommand::SelectAllOccurrences, "⌃⌘G"),
        (EditorCommand::AddNextOccurrence, "⌃G"),
        (EditorCommand::UnfoldAll, "⇧⌘="),
        (EditorCommand::GotoLine, "⌘L"),
        (EditorCommand::ContextActions, "⌥↩"),
        (EditorCommand::CopyReference, "⌥⇧⌘C"),
        (EditorCommand::Reformat, "⌥⌘L"),
        (EditorCommand::GotoDeclaration, "⌘B"),
        (EditorCommand::GotoImplementation, "⌥⌘B"),
        (EditorCommand::GotoTypeDeclaration, "⇧⌘B"),
        (EditorCommand::CallHierarchy, "⌃⌥H"),
        (EditorCommand::Completion, "⌃Space"),
        (EditorCommand::Rename, "⇧F6"),
        (EditorCommand::NextProblem, "F2"),
    ];
    for (command, expected) in mac {
        assert_eq!(label(command, true, true).as_deref(), Some(expected));
    }
    let linux = [
        (EditorCommand::DuplicateLine, "Ctrl+D"),
        (EditorCommand::DeleteLine, "Ctrl+Y"),
        (EditorCommand::MoveLineDown, "Alt+Shift+↓"),
        (EditorCommand::SelectAllOccurrences, "Ctrl+Alt+Shift+J"),
        (EditorCommand::AddNextOccurrence, "Alt+J"),
        (EditorCommand::GotoLine, "Ctrl+G"),
        (EditorCommand::ContextActions, "Alt+Enter"),
        (EditorCommand::CopyReference, "Ctrl+Alt+Shift+C"),
        (EditorCommand::CallHierarchy, "Ctrl+Alt+H"),
        (EditorCommand::Completion, "Ctrl+Space"),
    ];
    for (command, expected) in linux {
        assert_eq!(label(command, false, true).as_deref(), Some(expected));
    }
    // Without the JetBrains keymap the editor's other keys show.
    assert_eq!(
        label(EditorCommand::GotoDeclaration, true, false).as_deref(),
        Some("F12")
    );
    assert_eq!(
        label(EditorCommand::Rename, false, false).as_deref(),
        Some("F2")
    );
    assert_eq!(
        label(EditorCommand::PreviousProblem, true, false).as_deref(),
        Some("⇧F8")
    );
    assert_eq!(label(EditorCommand::GotoLine, true, false), None);
}

#[test]
fn every_command_has_its_own_name() {
    let mut names: Vec<_> = EditorCommand::ALL.iter().map(|c| c.name()).collect();
    assert!(names.iter().all(|name| !name.is_empty()));
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), EditorCommand::ALL.len());
}

#[test]
fn commands_apply_only_where_their_keys_work() {
    use EditorCommand as C;
    assert!(C::Rename.available(Language::Rust, false, false));
    assert!(!C::Rename.available(Language::Plain, false, false));
    assert!(C::AddNextOccurrence.available(Language::Plain, false, false));
    assert!(!C::AddNextOccurrence.available(Language::Plain, true, false));
    assert!(!C::CommentLine.available(Language::Json, false, false));
    // Read-only: navigation and the find bar stay; edits, code actions,
    // carets and typed prompts (Go to Line, vim search) do not.
    assert!(C::Find.available(Language::Rust, false, true));
    assert!(C::GotoDeclaration.available(Language::Rust, false, true));
    assert!(!C::DuplicateLine.available(Language::Rust, false, true));
    assert!(!C::ContextActions.available(Language::Rust, false, true));
    assert!(!C::AddNextOccurrence.available(Language::Rust, false, true));
    assert!(!C::GotoLine.available(Language::Rust, false, true));
    assert!(!C::Find.available(Language::Rust, true, true));
}

#[test]
fn a_queued_command_runs_once_in_its_own_view() {
    let mut editor = Editor::new(true);
    editor.frame(Vec::new());
    queue_command(&editor.ctx, "another-view", EditorCommand::DuplicateLine);
    editor.frame(Vec::new());
    assert_eq!(editor.doc.line_count(), 6, "another view's command waits");
    assert!(queued_command(&editor.ctx, "another-view").is_some());
    queue_command(&editor.ctx, ID, EditorCommand::DuplicateLine);
    editor.frame(Vec::new());
    editor.frame(Vec::new());
    assert_eq!(
        editor.doc.line_count(),
        7,
        "one duplicate, not one per frame"
    );
    assert_eq!(queued_command(&editor.ctx, ID), None);
}

#[test]
fn a_stale_queued_command_is_dropped() {
    let mut editor = Editor::new(true);
    let old = Instant::now()
        .checked_sub(Duration::from_secs(3))
        .expect("instant in range");
    editor.ctx.data_mut(|data| {
        data.insert_temp(queue_id(ID), (EditorCommand::DuplicateLine, old));
    });
    editor.frame(Vec::new());
    assert_eq!(editor.doc.text(), format!("{TEXT}// end\n"));
}

#[test]
fn a_queued_command_takes_focus_from_the_palette() {
    let mut editor = Editor::new(true);
    editor.frame(Vec::new());
    let palette = egui::Id::unique("palette-search");
    editor
        .ctx
        .memory_mut(|memory| memory.request_focus(palette));
    queue_command(&editor.ctx, ID, EditorCommand::GotoLine);
    editor.frame(Vec::new());
    assert!(
        editor
            .ctx
            .memory(|memory| memory.has_focus(source_focus_id(ID)))
    );
}

#[test]
fn read_only_and_vim_views_refuse_what_their_keys_refuse() {
    let mut editor = Editor::new(true);
    editor.options.read_only = true;
    let read_only = editor.run(Press::Queue(EditorCommand::DuplicateLine));
    assert_eq!(read_only.text, format!("{TEXT}// end\n"));
    let mut editor = Editor::new(true);
    editor.options.vim = true;
    let carets = editor.run(Press::Queue(EditorCommand::SelectAllOccurrences));
    assert_eq!(carets.carets, 0);
    // Vim keeps its own search prompt for Find.
    let mut editor = Editor::new(true);
    editor.options.vim = true;
    let find = editor.run(Press::Queue(EditorCommand::Find));
    assert_eq!(find.mode, Mode::Command);
    assert!(find.effects.is_empty());
}
