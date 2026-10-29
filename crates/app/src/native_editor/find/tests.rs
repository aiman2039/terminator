use super::*;

fn bar(query: &str) -> FindBar {
    FindBar::new(PathBuf::from("/f.rs"), query.into(), false)
}

#[test]
fn find_honors_case_word_and_regex() {
    let doc = Doc::new("Foo foo food\nfoo_bar FOO\n");
    let mut find = bar("foo");
    assert_eq!(find.matches(&doc).len(), 5);
    find.case = true;
    assert_eq!(find.matches(&doc).len(), 3);
    find.case = false;
    find.word = true;
    assert_eq!(find.matches(&doc).len(), 3);
    find.word = false;
    find.regex = true;
    find.query = "fo+d".into();
    assert_eq!(
        find.matches(&doc).as_slice(),
        [FindMatch {
            line: 0,
            start: 8,
            end: 12
        }]
    );
    find.query = "(".into();
    assert!(find.matches(&doc).is_empty());
    assert!(find.error.is_some());
}

#[test]
fn replace_all_is_one_undo_step_and_expands_groups() {
    let mut doc = Doc::new("let a = 1;\nlet b = 2;\n");
    let mut engine = VimEngine::new();
    let mut find = bar(r"let (\w)");
    find.regex = true;
    let matches = find.matches(&doc);
    let pattern = find.pattern().unwrap();
    replace_matches(ReplaceMatchesInput {
        buffer: &mut doc,
        engine: &mut engine,
        pattern: &pattern,
        replacement: "const $1",
        matches: &matches,
        regex: true,
    });
    assert_eq!(doc.text(), "const a = 1;\nconst b = 2;\n");
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "let a = 1;\nlet b = 2;\n");
}

#[test]
fn stepping_wraps_around_the_caret() {
    let matches = [
        FindMatch {
            line: 0,
            start: 0,
            end: 1,
        },
        FindMatch {
            line: 2,
            start: 4,
            end: 5,
        },
    ];
    assert_eq!(step(&matches, (1, 0), true), Some(1));
    assert_eq!(step(&matches, (3, 0), true), Some(0));
    assert_eq!(step(&matches, (1, 0), false), Some(0));
    assert_eq!(step(&matches, (0, 0), false), Some(1));
}

#[test]
fn in_selection_scopes_matches_and_replace_all_to_the_selection() {
    let mut doc = Doc::new("foo\nfoo foo\nfoo\n");
    let mut engine = VimEngine::new();
    let mut find = bar("foo");
    // Scope: the second "foo" on line 1.
    let from = doc.char_at_line_col(1, 4);
    find.scope = Some((from, from.saturating_add(3)));
    find.in_selection = true;
    assert_eq!(
        find.matches(&doc).as_slice(),
        [FindMatch {
            line: 1,
            start: 4,
            end: 7
        }]
    );
    find.replacement = "barbaz".into();
    let request = ReplaceRequest {
        one: false,
        all: true,
    };
    assert!(!find.apply_replace(&mut doc, &mut engine, request));
    assert_eq!(doc.text(), "foo\nfoo barbaz\nfoo\n");
    // The scope still covers the replaced text.
    assert_eq!(find.scope, Some((from, from.saturating_add(6))));
    // Turning the toggle off searches the whole buffer again.
    find.in_selection = false;
    assert_eq!(find.matches(&doc).len(), 3);
}

#[test]
fn a_multi_line_selection_sets_the_scope() {
    let mut doc = Doc::new("one\ntwo\nthree\n");
    let mut engine = VimEngine::new();
    let _ = engine.place_cursor(&mut doc, 0, 1);
    engine.extend_selection_to(&mut doc, 1, 2);
    let mut find = bar("");
    assert!(find.scope_to_selection(&engine, &doc));
    let (from, to) = find.scope.unwrap();
    assert_eq!(from, 1);
    assert!(to > doc.char_at_line_col(1, 0), "scope reaches line two");
    // No selection: nothing to scope to, the toggle stays off.
    let plain = VimEngine::new();
    assert!(!find.scope_to_selection(&plain, &doc));
    assert!(!find.in_selection);
}
