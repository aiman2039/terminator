use super::signature::open_paren;
use super::*;

#[test]
fn the_completion_list_goes_above_the_caret_line_near_the_pane_bottom() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let caret = |top: f32| egui::Rect::from_min_size(egui::pos2(100.0, top), egui::vec2(2.0, 18.0));
    // Room below: under the caret line, every row.
    let (at, pivot, rows) = super::popup::list_place(caret(100.0), screen, 30);
    assert_eq!(
        (at, pivot, rows),
        (
            caret(100.0).left_bottom(),
            egui::Align2::LEFT_TOP,
            VISIBLE_ROWS
        )
    );
    // Near the bottom: above the caret line, so the typed line stays clear.
    let near = caret(560.0);
    let (at, pivot, rows) = super::popup::list_place(near, screen, 30);
    assert_eq!((at, pivot), (near.left_top(), egui::Align2::LEFT_BOTTOM));
    assert_eq!(rows, VISIBLE_ROWS);
    // A short pane shows only the rows that fit.
    let short = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 200.0));
    let (_, _, rows) = super::popup::list_place(caret(90.0), short, 30);
    assert!((1..VISIBLE_ROWS).contains(&rows), "{rows}");
}

fn item(label: &str, sort: &str) -> native_lsp::CompletionItem {
    native_lsp::CompletionItem {
        label: label.into(),
        kind: Some("Function".into()),
        insert: label.into(),
        filter: label.into(),
        sort: sort.into(),
        raw: serde_json::json!({ "label": label }),
        ..Default::default()
    }
}

#[test]
fn signature_opens_inside_calls_and_closes_outside() {
    assert_eq!(open_paren("f(a, g(b), ", 11), Some(2));
    assert_eq!(open_paren("f(a)", 4), None);
    let mut buffer = Doc::new("f(\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 0, 2);
    engine.modeless_key(&mut buffer, terminator_native_edit::vim::Key::Char('x'));
    let before = buffer.revision();
    engine.type_text(&mut buffer, ",");
    let mut popup = None;
    let asked = signature_after_frame(SignatureAfterFrameInput {
        popup: &mut popup,
        buffer: &buffer,
        engine: &engine,
        revision_before: before,
        server: true,
    });
    assert_eq!(asked.map(|(_, line, open)| (line, open)), Some((0, 2)));
    popup = Some(SignaturePopup {
        path: PathBuf::from("/f.rs"),
        generation: 1,
        help: None,
        line: 0,
        open: 2,
        anchor: None,
    });
    // Leaving the call (past its `)`) closes the popup.
    let _ = engine.place_cursor(&mut buffer, 0, 0);
    let rev = buffer.revision();
    assert!(
        signature_after_frame(SignatureAfterFrameInput {
            popup: &mut popup,
            buffer: &buffer,
            engine: &engine,
            revision_before: rev,
            server: true
        })
        .is_none()
    );
    assert!(popup.is_none());
}

#[test]
fn prefix_and_trigger_rules() {
    assert_eq!(prefix_start("let foo_ba", 10), 4);
    let invoked = Some(CompletionTrigger::Invoked);
    assert_eq!(trigger_at("x.", 2, &[]), invoked);
    assert_eq!(trigger_at("std::", 5, &[]), invoked);
    assert_eq!(trigger_at("a :", 3, &[]), None);
    assert_eq!(trigger_at("a ", 2, &[]), None);
}

#[test]
fn server_trigger_characters_open_completion() {
    use CompletionTrigger::{Character, Invoked};
    // rust-analyzer's list.
    let rust = [':', '.', '\'', '('];
    assert_eq!(trigger_at("x.", 2, &rust), Some(Character('.')));
    assert_eq!(trigger_at("a :", 3, &rust), Some(Character(':')));
    assert_eq!(trigger_at("f(", 2, &rust), Some(Character('(')));
    assert_eq!(
        trigger_at("ab", 2, &rust),
        Some(Invoked),
        "identifiers invoke"
    );
    assert_eq!(trigger_at("", 0, &rust), None);
    // A server's own list replaces the built-in `.` and `::`.
    assert_eq!(trigger_at("x.", 2, &['<']), None);
    assert_eq!(trigger_at("Vec<", 4, &['<']), Some(Character('<')));
}

#[test]
fn ranking_prefers_prefix_matches_then_server_order() {
    let items = [
        item("to_string", "2"),
        item("Tokio", "1"),
        item("into_iter", "0"),
        item("as_ref", "0"),
    ];
    // `to`: exact-case prefix, then any-case prefix, then subsequence.
    assert_eq!(rank(&items, "to"), [0, 1, 2]);
}

#[test]
fn accepting_replaces_the_prefix_and_lands_on_the_snippet_stop() {
    let mut buffer = Doc::new("let v = pri\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 0, 11);
    let mut chosen = item("println!", "0");
    chosen.insert = "println!(\"$1\")$0".into();
    chosen.snippet = true;
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        path: Path::new("/f.rs"),
        line: 0,
        start: 8,
        item: &chosen,
    });
    assert_eq!(buffer.text(), "let v = println!(\"\")\n");
    assert_eq!(engine.cursor().col, 18);
    engine.undo(&mut buffer);
    assert_eq!(buffer.text(), "let v = pri\n");
}

#[test]
fn accepting_a_snippet_selects_its_first_stop_until_tab_moves_on() {
    let mut buffer = Doc::new("let p = pa\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 0, 10);
    let mut chosen = item("path", "0");
    chosen.insert = "${1:dir}.join(\"$TM_FILENAME\")$0".into();
    chosen.snippet = true;
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        path: Path::new("/src/main.rs"),
        line: 0,
        start: 8,
        item: &chosen,
    });
    assert_eq!(buffer.text(), "let p = dir.join(\"main.rs\")\n");
    assert_eq!(
        engine.copy_text(&buffer),
        "dir",
        "the first stop is selected"
    );
    assert!(engine.snippet_jump(&mut buffer, true));
    assert_eq!(engine.cursor().col, 27, "Tab ends on `$0`");
    assert!(!engine.snippet_active());
    // A plain item inside a running session keeps it going.
    let mut buffer = Doc::new("f(\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let snippet = native_lsp::parse_snippet("${1:x}, ${2:y})", &native_lsp::SnippetVars::default());
    buffer.insert(2, &snippet.text);
    engine.start_snippet(&mut buffer, 2, &snippet);
    engine.modeless_key(&mut buffer, terminator_native_edit::vim::Key::Char('l'));
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        path: Path::new("/f.rs"),
        line: 0,
        start: 2,
        item: &item("len", "0"),
    });
    assert_eq!(buffer.text(), "f(len, y)\n");
    assert!(engine.snippet_jump(&mut buffer, true));
    assert_eq!(engine.copy_text(&buffer), "y");
}

#[test]
fn completion_badges_match_the_structure_view() {
    use super::popup::kind_badge;
    assert_eq!(kind_badge(Some("Struct")).map(|(c, _)| c), Some('S'));
    assert_eq!(kind_badge(Some("Method")).map(|(c, _)| c), Some('m'));
    assert_eq!(kind_badge(Some("Field")).map(|(c, _)| c), Some('p'));
    // Keywords show no badge, as in `IntelliJ`.
    assert!(kind_badge(Some("Keyword")).is_none());
    assert!(kind_badge(None).is_none());
    // Functions and fields read as different groups.
    assert_ne!(
        kind_badge(Some("Function")).map(|(_, tint)| tint),
        kind_badge(Some("Field")).map(|(_, tint)| tint)
    );
}

#[test]
fn accepting_applies_auto_imports_and_the_replace_range() {
    let position = |line, character| native_lsp::Position { line, character };
    let mut buffer = Doc::new("fn main() {\n    HashSe\n}\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 1, 8);
    let mut chosen = item("HashSet", "0");
    // Caret inside the word: the replace range covers its tail too.
    chosen.replace = Some(native_lsp::Range {
        start: position(1, 4),
        end: position(1, 10),
    });
    chosen.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: position(0, 0),
            end: position(0, 0),
        },
        new_text: "use std::collections::HashSet;\n".into(),
    }];
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        path: Path::new("/f.rs"),
        line: 1,
        start: 4,
        item: &chosen,
    });
    assert_eq!(
        buffer.text(),
        "use std::collections::HashSet;\nfn main() {\n    HashSet\n}\n"
    );
    assert_eq!(
        (engine.cursor().line, engine.cursor().col),
        (2, 11),
        "the caret follows the inserted item"
    );
    engine.undo(&mut buffer);
    assert_eq!(buffer.text(), "fn main() {\n    HashSe\n}\n");
}

/// A modeless editor typing on one line, with its completion popup.
struct Typing {
    buffer: Doc,
    engine: VimEngine,
    popup: Option<CompletionPopup>,
}

impl Typing {
    fn new(text: &str, (line, col): (usize, usize)) -> Self {
        let mut buffer = Doc::new(text);
        let mut engine = VimEngine::new();
        engine.set_modeless(true);
        // Typing happens in Insert mode, as a modeless view starts.
        engine.press_key(&mut buffer, terminator_native_edit::vim::Key::Char('i'));
        let _ = engine.place_cursor(&mut buffer, line, col);
        Self {
            buffer,
            engine,
            popup: None,
        }
    }

    /// Show `items` as the answer to request 1, for the prefix at `start`.
    fn answered(&mut self, start: usize, items: Vec<native_lsp::CompletionItem>, incomplete: bool) {
        let path = PathBuf::from("/f.rs");
        let line = self.engine.cursor().line;
        let mut popup = CompletionPopup::new(&path, 1, (line, start), None);
        popup.adopt(CompletionResults {
            path,
            generation: 1,
            items,
            incomplete,
        });
        self.popup = Some(popup);
    }

    /// Type `text`; returns the buffer revision before it.
    fn type_text(&mut self, text: &str) -> u64 {
        let before = self.buffer.revision();
        self.engine.type_text(&mut self.buffer, text);
        before
    }

    /// One frame of completion upkeep after the edits since `before`,
    /// with `keys` pressed.
    fn frame(&mut self, before: u64, keys: &[egui::Key], triggers: &[char]) -> AfterFrame {
        let events = keys
            .iter()
            .map(|key| egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect();
        let Self {
            buffer,
            engine,
            popup,
        } = self;
        let mut out = None;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = egui::Context::default().run_ui(input, |ui| {
            if out.is_none() {
                out = Some(after_frame(AfterFrameInput {
                    ui,
                    popup: &mut *popup,
                    buffer: &mut *buffer,
                    engine: &mut *engine,
                    revision_before: before,
                    manual: false,
                    caret_rect: None,
                    server: true,
                    triggers,
                }));
            }
        });
        output.textures_delta.clear();
        out.unwrap_or_default()
    }
}

#[test]
fn typing_on_an_incomplete_list_asks_again_and_keeps_it_shown() {
    let mut typing = Typing::new("let v = pri\n", (0, 11));
    typing.answered(8, vec![item("print", "0"), item("println", "1")], true);
    let before = typing.type_text("n");
    let ask = typing.frame(before, &[], &[]).ask.expect("asks again");
    assert_eq!(ask.trigger, CompletionTrigger::Incomplete);
    assert_eq!((ask.line, ask.start, ask.position.character), (0, 8, 12));
    let popup = typing.popup.as_ref().expect("still open");
    assert_eq!(
        popup.items.len(),
        2,
        "the old list shows until the new one lands"
    );
    assert_eq!(popup.prefix, "prin");
    // A frame without an edit asks nothing.
    let now = typing.buffer.revision();
    assert!(typing.frame(now, &[], &[]).ask.is_none());
}

#[test]
fn a_complete_list_only_filters_as_the_user_types() {
    let mut typing = Typing::new("let v = pri\n", (0, 11));
    let items = vec![
        item("print", "0"),
        item("println", "1"),
        item("private", "2"),
    ];
    typing.answered(8, items, false);
    let before = typing.type_text("n");
    assert!(typing.frame(before, &[], &[]).ask.is_none());
    assert_eq!(typing.popup.as_ref().expect("open").visible, [0, 1]);
    // A trigger character closes the list and asks for a new one.
    let before = typing.type_text(".");
    let ask = typing.frame(before, &[], &['.']).ask.expect("trigger asks");
    assert_eq!(ask.trigger, CompletionTrigger::Character('.'));
    assert_eq!(ask.start, 13);
    assert!(typing.popup.is_none());
}

#[test]
fn accepting_an_unresolved_item_asks_for_its_imports() {
    let accept_first = |chosen: native_lsp::CompletionItem| {
        let mut typing = Typing::new("let s = HashSe\n", (0, 14));
        typing.answered(8, vec![chosen], true);
        let now = typing.buffer.revision();
        let after = typing.frame(now, &[egui::Key::Enter], &[]);
        (typing, after)
    };
    let mut set = item("HashSet", "0");
    set.raw = serde_json::json!({"label": "HashSet", "data": {"imports": ["std::collections"]}});
    let (typing, after) = accept_first(set.clone());
    assert_eq!(typing.buffer.text(), "let s = HashSet\n");
    assert!(typing.popup.is_none());
    let accepted = after.accepted.expect("its imports are resolved");
    assert_eq!(accepted.raw, set.raw);
    let before = docs::text_before_hash(&typing.buffer, 0, 8);
    assert_eq!(
        accepted.target,
        ResolveFor::Accepted {
            line: 0,
            col: 8,
            before
        }
    );
    // A resolve that found no edits may have run while the server still
    // loaded: accepting asks again. An item with its edits needs nothing.
    let mut resolved = set.clone();
    resolved.resolved = true;
    assert!(accept_first(resolved).1.accepted.is_some());
    let mut eager = set;
    eager.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: native_lsp::Position {
                line: 0,
                character: 0,
            },
            end: native_lsp::Position {
                line: 0,
                character: 0,
            },
        },
        new_text: "use std::collections::HashSet;\n".into(),
    }];
    let (typing, after) = accept_first(eager);
    assert!(after.accepted.is_none());
    assert!(
        typing
            .buffer
            .text()
            .starts_with("use std::collections::HashSet;\n")
    );
}

/// A popup answered by request 1 with `items`, for an empty prefix.
fn listed(items: Vec<native_lsp::CompletionItem>) -> CompletionPopup {
    let path = PathBuf::from("/f.rs");
    let mut popup = CompletionPopup::new(&path, 1, (0, 0), None);
    popup.adopt(CompletionResults {
        path,
        generation: 1,
        items,
        incomplete: false,
    });
    popup
}

fn documented(label: &str, sort: &str) -> native_lsp::CompletionItem {
    let mut item = item(label, sort);
    item.documentation = Some(native_lsp::Hover {
        text: format!("Docs for `{label}`."),
        markdown: true,
    });
    item
}

#[test]
fn the_selection_is_resolved_once_it_rests_without_docs() {
    let mut popup = listed(vec![
        item("push", "0"),
        documented("len", "1"),
        item("pop", "2"),
    ]);
    let start = Instant::now();
    let at = |millis| {
        start
            .checked_add(Duration::from_millis(millis))
            .expect("time")
    };
    assert_eq!(
        resolve_due(&mut popup, start),
        Due::Later(docs::RESOLVE_DELAY)
    );
    assert_eq!(
        resolve_due(&mut popup, at(40)),
        Due::Later(Duration::from_millis(60))
    );
    let rested = start.checked_add(docs::RESOLVE_DELAY).expect("rested");
    assert_eq!(resolve_due(&mut popup, rested), Due::Now(0));
    assert_eq!(resolve_due(&mut popup, rested), Due::Never, "asked once");
    // An item with docs needs no lookup.
    popup.selected = 1;
    assert_eq!(resolve_due(&mut popup, rested), Due::Never);
    // Moving the selection restarts the wait.
    popup.selected = 2;
    assert_eq!(
        resolve_due(&mut popup, rested),
        Due::Later(docs::RESOLVE_DELAY)
    );
    let later = rested.checked_add(docs::RESOLVE_DELAY).expect("later");
    // Not while a newer list is on its way: it replaces these items.
    popup.generation = Some(2);
    assert_eq!(resolve_due(&mut popup, later), Due::Never);
    popup.generation = None;
    assert_eq!(resolve_due(&mut popup, later), Due::Now(2));
}

#[test]
fn resolve_answers_merge_into_their_own_list_only() {
    let mut popup = listed(vec![item("len", "0")]);
    let mut answer = documented("len", "0");
    answer.detail = Some("fn(&self) -> usize".into());
    popup.merge_resolved((7, 0), answer.clone());
    assert!(
        popup.items[0].documentation.is_none(),
        "an older list's answer"
    );
    popup.waiting = Some(Instant::now());
    popup.merge_resolved((1, 0), answer);
    assert!(popup.items[0].resolved);
    assert_eq!(popup.items[0].detail.as_deref(), Some("fn(&self) -> usize"));
    assert!(popup.items[0].documentation.is_some());
    assert!(popup.waiting.is_none(), "the frame stops looking");
}

#[test]
fn a_new_list_keeps_resolved_docs_and_the_selection() {
    let mut set = documented("HashSet", "2");
    set.resolved = true;
    set.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: native_lsp::Position {
                line: 0,
                character: 0,
            },
            end: native_lsp::Position {
                line: 0,
                character: 0,
            },
        },
        new_text: "use std::collections::HashSet;\n".into(),
    }];
    let mut popup = listed(vec![item("as_ref", "0"), documented("len", "1"), set]);
    popup.selected = 1;
    popup.generation = Some(2);
    let results = CompletionResults {
        path: PathBuf::from("/f.rs"),
        generation: 2,
        items: vec![
            item("abs", "0"),
            item("as_ref", "1"),
            item("len", "2"),
            item("HashSet", "3"),
        ],
        incomplete: true,
    };
    popup.adopt(results);
    assert_eq!(
        (popup.list, popup.generation, popup.incomplete),
        (2, None, true)
    );
    let selected = popup.visible[popup.selected];
    assert_eq!(popup.items[selected].label, "len", "the selection stays");
    assert!(
        popup.items[selected].documentation.is_some(),
        "docs carry over"
    );
    assert!(
        !popup.items[selected].resolved,
        "its edits are still to resolve"
    );
    assert!(popup.items[1].documentation.is_none());
    // A resolved item keeps its import edits: it is not asked again.
    let set = &popup.items[3];
    assert!(set.resolved);
    assert_eq!(set.additional.len(), 1);
}

#[test]
fn late_imports_apply_only_while_the_text_before_is_unchanged() {
    let at = |line, character| native_lsp::Position { line, character };
    let edit = |(start, end): (native_lsp::Position, native_lsp::Position), text: &str| {
        native_lsp::RenameTextEdit {
            range: native_lsp::Range { start, end },
            new_text: text.into(),
        }
    };
    let mut buffer = Doc::new("fn main() {\n    let s = HashSet\n}\n");
    let accepted = (1, 12, docs::text_before_hash(&buffer, 1, 12));
    let import = [edit(
        (at(0, 0), at(0, 0)),
        "use std::collections::HashSet;\n",
    )];
    assert!(imports_fit(&buffer, accepted, &import));
    // Typing on after the item keeps them valid.
    buffer.insert(buffer.char_at_line_col(1, 19), "::new()");
    assert!(imports_fit(&buffer, accepted, &import));
    // An edit that reaches past the insert point does not fit.
    assert!(!imports_fit(
        &buffer,
        accepted,
        &[edit((at(1, 4), at(1, 14)), "x")]
    ));
    // A change above the insert point moves everything: they are dropped.
    buffer.insert(0, "// note\n");
    assert!(!imports_fit(&buffer, accepted, &import));
}

#[test]
fn the_docs_panel_goes_right_of_the_list_or_left_without_room() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let list = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(520.0, 300.0));
    let (at, pivot) = docs::docs_place(list, screen);
    assert_eq!(pivot, egui::Align2::LEFT_TOP);
    assert!(at.x > list.right());
    assert_eq!(at.y, list.top());
    let near_edge = egui::Rect::from_min_size(egui::pos2(600.0, 200.0), egui::vec2(520.0, 300.0));
    let (at, pivot) = docs::docs_place(near_edge, screen);
    assert_eq!(pivot, egui::Align2::RIGHT_TOP);
    assert!(at.x < near_edge.left());
}

#[test]
fn rows_show_the_label_description_or_else_the_detail() {
    use super::popup::right_text;
    let mut row = item("len", "0");
    assert_eq!(right_text(&row), None);
    row.detail = Some("fn(&self) -> usize".into());
    assert_eq!(right_text(&row), Some("fn(&self) -> usize"));
    row.label_description = Some("usize".into());
    assert_eq!(right_text(&row), Some("usize"), "label details win");
}
